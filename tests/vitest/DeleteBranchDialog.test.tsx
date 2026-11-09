// @vitest-environment jsdom
//
// 组件测试只在这个文件里切到 jsdom，其余纯逻辑测试仍跑在 node 环境（vitest.config.ts）。
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { DeleteBranchDialog, type DeleteInfo } from '@/components/DeleteBranchDialog'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import type { BranchDeletionStep } from '@/engine/deleteBranchTargets'
import type { PullRequest } from '@/engine/types'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const invokeMock = vi.mocked(invoke)

afterEach(() => {
  invokeMock.mockReset()
})

const INFO: DeleteInfo = {
  upstream: null,
  remote: null,
  remoteBranch: null,
  remoteProtected: false,
  unmerged: 0,
  comparedTo: 'main',
  contentMerged: false,
  mergedVia: null,
  mergedInto: null,
  worktree: null,
}

const MERGED_PR: PullRequest = {
  number: 7,
  headRefName: 'feat/x',
  title: 't',
  isDraft: false,
  state: 'MERGED',
  headRefOid: 'a'.repeat(40),
  url: 'https://example.invalid/pr/7',
}

type Targets = { remote: unknown; worktreePath: string | null; staleRemote?: unknown }
type OnStepDone = (step: BranchDeletionStep) => void

/** 一个手动放行的 Promise：用来让「带合并点的重查」停在半路。 */
function deferred<T>() {
  let resolve!: (v: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

function dialog(props: { prs: PullRequest[]; onConfirm?: () => Promise<void> }) {
  return (
    <DeleteBranchDialog
      repoPath="C:\\repo"
      name="feat/x"
      capabilities={ALL_CAPABILITIES}
      mergedPullRequests={props.prs}
      onCancel={() => {}}
      onConfirm={props.onConfirm ?? (async () => {})}
    />
  )
}

function deleteButton(): HTMLButtonElement {
  return screen.getByRole('button', { name: 'Delete' }) as HTMLButtonElement
}

describe('DeleteBranchDialog', () => {
  it('PR 列表晚到触发的重查返回时，不能清掉删除失败的错误（PR 56 Codex review）', async () => {
    // 真实时序：弹框先不带合并点查一次；PR 列表随后到达，带着合并点再查一次，
    // 这一次还在路上时用户点了删除、删除失败——删除按 worktree → 远程 → 本地执行，
    // 失败时可能已经做了一半，这条错误是用户唯一的线索
    const requery = deferred<DeleteInfo>()
    invokeMock.mockResolvedValueOnce(INFO).mockReturnValueOnce(requery.promise)
    const onConfirm = vi.fn(async () => {
      throw new Error('remote rejected')
    })

    const { rerender } = render(dialog({ prs: [], onConfirm }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))

    rerender(dialog({ prs: [MERGED_PR], onConfirm }))
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(2))

    fireEvent.click(deleteButton())
    expect(await screen.findByText(/remote rejected/)).toBeTruthy()

    await act(async () => {
      requery.resolve(INFO)
      await requery.promise
    })
    expect(screen.queryByText(/remote rejected/)).not.toBeNull()
  })

  it('第一次查询失败、PR 列表晚到后重查成功：旧的查询错误要清掉', async () => {
    invokeMock.mockRejectedValueOnce(new Error('branch info failed')).mockResolvedValueOnce(INFO)

    const { rerender } = render(dialog({ prs: [] }))
    expect(await screen.findByText(/branch info failed/)).toBeTruthy()

    rerender(dialog({ prs: [MERGED_PR] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(screen.queryByText(/branch info failed/)).toBeNull()
  })

  const BOTH_TARGETS: DeleteInfo = {
    ...INFO,
    upstream: 'origin/feat/x',
    remote: 'origin',
    remoteBranch: 'feat/x',
    worktree: { name: 'wt', path: 'C:/wt' },
  }

  function retryDialog(onConfirm: (targets: Targets, onStepDone: OnStepDone) => Promise<void>) {
    return (
      <DeleteBranchDialog
        repoPath="C:\\repo"
        name="feat/x"
        capabilities={ALL_CAPABILITIES}
        mergedPullRequests={[]}
        onCancel={() => {}}
        onConfirm={onConfirm}
      />
    )
  }

  it('worktree 删完、push 失败：重试不再删 worktree，远程照旧，勾选框也消失', async () => {
    // 不记进度的话重试还会带上旧的 worktreePath，后端报 worktree_not_registered，
    // 序列永远走不到远程和本地
    invokeMock.mockResolvedValue(BOTH_TARGETS)
    const onConfirm = vi
      .fn<(targets: Targets, onStepDone: OnStepDone) => Promise<void>>()
      .mockImplementationOnce(async (_targets, onStepDone) => {
        onStepDone('worktree')
        throw new Error('network down')
      })
      .mockResolvedValue(undefined)

    render(retryDialog(onConfirm))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(screen.getByRole('checkbox', { name: /Also delete remote branch/ }))

    fireEvent.click(deleteButton())
    expect(await screen.findByText(/network down/)).toBeTruthy()
    expect(onConfirm.mock.calls[0][0].worktreePath).toBe('C:/wt')
    expect(screen.queryByText('Also delete worktree')).toBeNull()
    expect(screen.getByText(/已经删除：worktree/)).toBeTruthy()

    fireEvent.click(deleteButton())
    await waitFor(() => expect(onConfirm).toHaveBeenCalledTimes(2))
    expect(onConfirm.mock.calls[1][0]).toEqual({
      remote: { remote: 'origin', branch: 'feat/x' },
      worktreePath: null,
      staleRemote: null,
    })
  })

  it('远程删完、本地失败：重试不再 push --delete，否则 ref 不存在会一直卡在这一步', async () => {
    invokeMock.mockResolvedValue({ ...BOTH_TARGETS, worktree: null })
    const onConfirm = vi
      .fn<(targets: Targets, onStepDone: OnStepDone) => Promise<void>>()
      .mockImplementationOnce(async (_targets, onStepDone) => {
        onStepDone('remote')
        throw new Error('local delete failed')
      })
      .mockResolvedValue(undefined)

    render(retryDialog(onConfirm))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(screen.getByRole('checkbox', { name: /Also delete remote branch/ }))

    fireEvent.click(deleteButton())
    expect(await screen.findByText(/local delete failed/)).toBeTruthy()
    expect(screen.queryByRole('checkbox', { name: /Also delete remote branch/ })).toBeNull()

    fireEvent.click(deleteButton())
    await waitFor(() => expect(onConfirm).toHaveBeenCalledTimes(2))
    expect(onConfirm.mock.calls[1][0].remote).toBeNull()
  })

  it('远程删完后 PR 列表晚到的重查扫到别的远程：绝不能改删那条用户没选过的分支', async () => {
    // 真实时序：feat/x 在 origin 和 upstream 上都有。origin 那条删掉之后再查，
    // find_remote_branch 会扫到 upstream/feat/x，勾选状态还在，而护栏只拦默认分支
    const onOrigin: DeleteInfo = { ...BOTH_TARGETS, worktree: null }
    invokeMock.mockResolvedValueOnce(onOrigin).mockResolvedValue({
      ...onOrigin,
      upstream: 'upstream/feat/x',
      remote: 'upstream',
    })
    const onConfirm = vi
      .fn<(targets: Targets, onStepDone: OnStepDone) => Promise<void>>()
      .mockImplementationOnce(async (_targets, onStepDone) => {
        onStepDone('remote')
        throw new Error('local delete failed')
      })
      .mockResolvedValue(undefined)
    const withPrs = (prs: PullRequest[]) => (
      <DeleteBranchDialog
        repoPath="C:\\repo"
        name="feat/x"
        capabilities={ALL_CAPABILITIES}
        mergedPullRequests={prs}
        onCancel={() => {}}
        onConfirm={onConfirm}
      />
    )

    const { rerender } = render(withPrs([]))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(screen.getByRole('checkbox', { name: /Also delete remote branch/ }))
    fireEvent.click(deleteButton())
    expect(await screen.findByText(/local delete failed/)).toBeTruthy()

    rerender(withPrs([MERGED_PR]))
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(3))

    fireEvent.click(deleteButton())
    await waitFor(() => expect(onConfirm).toHaveBeenCalledTimes(2))
    expect(onConfirm.mock.calls[1][0].remote).toBeNull()
  })
})

describe('DeleteBranchDialog 勾选远程时探测（顺带预取默认分支）', () => {
  const WITH_REMOTE: DeleteInfo = {
    ...INFO,
    upstream: 'origin/feat/x',
    remote: 'origin',
    remoteBranch: 'feat/x',
  }

  /** 按命令名分派：`branch_delete_info` 回 `info`，探测回 `present`（默认「还在」）。 */
  function respondWith(info: DeleteInfo, present: boolean | null = true) {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'branch_delete_info' ? info : cmd === 'probe_remote_branch' ? present : undefined)
  }

  function probeCalls() {
    return invokeMock.mock.calls.filter(([cmd]) => cmd === 'probe_remote_branch')
  }

  function remoteCheckbox(): HTMLInputElement {
    return screen.getByRole('checkbox', { name: /Also delete remote branch/ }) as HTMLInputElement
  }

  it('只删本地（远程那项没勾）：不走网络——SSH agent 审批、凭据框都不该为此弹出来', async () => {
    respondWith(WITH_REMOTE)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(remoteCheckbox().checked).toBe(false)
    expect(probeCalls()).toHaveLength(0)
  })

  it('勾上远程那项就预取，护栏在点确认时不必再握一次手', async () => {
    respondWith(WITH_REMOTE)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(probeCalls()).toEqual([
      // JSX 属性字符串不处理转义：dialog() 里的 repoPath="C:\\repo" 实际是两个反斜杠
      ['probe_remote_branch', { path: String.raw`C:\\repo`, remote: 'origin', branch: 'feat/x' }],
    ]))
  })

  it('探测答「远程上已经没有」：勾选框取消并灰掉、说明原因，删除只删本地', async () => {
    respondWith(WITH_REMOTE, false)
    const onConfirm = vi.fn(async (_targets: Targets) => {})
    render(
      <DeleteBranchDialog
        repoPath="C:\\repo"
        name="feat/x"
        capabilities={ALL_CAPABILITIES}
        mergedPullRequests={[]}
        onCancel={() => {}}
        onConfirm={onConfirm}
      />,
    )
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())

    expect(await screen.findByText(/远程上已经没有这条分支了/)).toBeTruthy()
    expect(remoteCheckbox().checked).toBe(false)
    expect(remoteCheckbox().disabled).toBe(true)

    fireEvent.click(deleteButton())
    await waitFor(() => expect(onConfirm).toHaveBeenCalledTimes(1))
    expect(onConfirm.mock.calls[0][0].remote).toBeNull()
    // 过期的跟踪 ref 交给删除序列在最后清，探测本身不碰它
    expect(onConfirm.mock.calls[0][0].staleRemote).toEqual({ remote: 'origin', branch: 'feat/x' })
  })

  it('探测在删除途中才答「没有」：不用这个答案，勾选框和进度文案都不变', async () => {
    const probe = deferred<boolean | null>()
    const deletion = deferred<void>()
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'branch_delete_info' ? WITH_REMOTE : cmd === 'probe_remote_branch' ? probe.promise : undefined)
    render(dialog({ prs: [], onConfirm: () => deletion.promise }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    fireEvent.click(deleteButton())
    expect(await screen.findByText('正在联系远程…')).toBeTruthy()

    await act(async () => {
      probe.resolve(false)
      await probe.promise
    })
    expect(screen.getByText('正在联系远程…')).toBeTruthy()
    // push 已经发出去了：这时改勾选框，push 若因别的原因失败，重试会悄悄跳过远程
    expect(remoteCheckbox().checked).toBe(true)
    expect(screen.queryByText(/远程上已经没有这条分支了/)).toBeNull()

    await act(async () => {
      deletion.resolve()
      await deletion.promise
    })
  })

  it('探测问不到（null）：不下结论，勾选照旧，删除照样带上远程', async () => {
    respondWith(WITH_REMOTE, null)
    const onConfirm = vi.fn(async (_targets: Targets) => {})
    render(
      <DeleteBranchDialog
        repoPath="C:\\repo"
        name="feat/x"
        capabilities={ALL_CAPABILITIES}
        mergedPullRequests={[]}
        onCancel={() => {}}
        onConfirm={onConfirm}
      />,
    )
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(probeCalls()).toHaveLength(1))

    fireEvent.click(deleteButton())
    await waitFor(() => expect(onConfirm).toHaveBeenCalledTimes(1))
    expect(onConfirm.mock.calls[0][0].remote).toEqual({ remote: 'origin', branch: 'feat/x' })
    expect(onConfirm.mock.calls[0][0].staleRemote).toBeNull()
    expect(screen.queryByText(/远程上已经没有这条分支了/)).toBeNull()
  })

  it('探测还在路上时用户取消了勾选：答案回来也不再改界面', async () => {
    const probe = deferred<boolean | null>()
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'branch_delete_info' ? WITH_REMOTE : cmd === 'probe_remote_branch' ? probe.promise : undefined)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(probeCalls()).toHaveLength(1))
    fireEvent.click(remoteCheckbox())

    await act(async () => {
      probe.resolve(false)
      await probe.promise
    })
    expect(screen.queryByText(/远程上已经没有这条分支了/)).toBeNull()
    expect(remoteCheckbox().disabled).toBe(false)
  })

  it('重查换了一条远程分支：上一条「已经没有」的答案不能沿用到新的那条上', async () => {
    let current = WITH_REMOTE
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'branch_delete_info' ? current : cmd === 'probe_remote_branch' ? false : undefined)
    const { rerender } = render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    expect(await screen.findByText(/远程上已经没有这条分支了/)).toBeTruthy()

    current = { ...WITH_REMOTE, upstream: 'upstream/feat/x', remote: 'upstream' }
    rerender(dialog({ prs: [MERGED_PR] }))
    await waitFor(() => expect(screen.getByText('upstream/feat/x')).toBeTruthy())
    expect(screen.queryByText(/远程上已经没有这条分支了/)).toBeNull()
    expect(remoteCheckbox().disabled).toBe(false)
  })

  it('PR 列表晚到触发重查：远程没变，不再发一趟', async () => {
    respondWith(WITH_REMOTE)
    const { rerender } = render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(probeCalls()).toHaveLength(1))
    rerender(dialog({ prs: [MERGED_PR] }))
    await waitFor(() =>
      expect(invokeMock.mock.calls.filter(([c]) => c === 'branch_delete_info')).toHaveLength(2))
    expect(probeCalls()).toHaveLength(1)
  })

  it('勾着的时候重查发现远程分支受保护：换了远程也不预取（门控看的是能不能删，不只是勾没勾）', async () => {
    let current = WITH_REMOTE
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'branch_delete_info' ? current : undefined)
    const { rerender } = render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(probeCalls()).toHaveLength(1))

    current = { ...WITH_REMOTE, remote: 'upstream', remoteProtected: true }
    rerender(dialog({ prs: [MERGED_PR] }))
    await waitFor(() => expect(remoteCheckbox().disabled).toBe(true))
    expect(probeCalls()).toHaveLength(1)
  })

  it('探测还在路上时取消再勾选：复用那次探测，不再握一次手', async () => {
    const probe = deferred<boolean | null>()
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'branch_delete_info' ? WITH_REMOTE : cmd === 'probe_remote_branch' ? probe.promise : undefined)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    fireEvent.click(remoteCheckbox())
    fireEvent.click(remoteCheckbox())

    // 复用的那次探测答了「没有」，重新勾上之后照样生效
    await act(async () => {
      probe.resolve(false)
      await probe.promise
    })
    expect(probeCalls()).toHaveLength(1)
    expect(screen.getByText(/远程上已经没有这条分支了/)).toBeTruthy()
  })

  it('上次探测答「还在」：取消再勾选不再探测', async () => {
    respondWith(WITH_REMOTE, true)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(probeCalls()).toHaveLength(1))
    await act(async () => {})
    fireEvent.click(remoteCheckbox())
    fireEvent.click(remoteCheckbox())
    await act(async () => {})
    expect(probeCalls()).toHaveLength(1)
  })

  it('上次探测问不到：取消再勾选重新探测（断网恢复后靠这个刷新）', async () => {
    respondWith(WITH_REMOTE, null)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(probeCalls()).toHaveLength(1))
    await act(async () => {})
    fireEvent.click(remoteCheckbox())
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(probeCalls()).toHaveLength(2))
  })

  it('默认分支的勾选框是禁用的，点不上，也就不会预取', async () => {
    respondWith({ ...WITH_REMOTE, remoteProtected: true })
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(remoteCheckbox().disabled).toBe(true)
    fireEvent.click(remoteCheckbox())
    expect(remoteCheckbox().checked).toBe(false)
    expect(probeCalls()).toHaveLength(0)
  })

  it('不管理远程的模式（SVN 混合仓库）没有勾选框，也就不会预取', async () => {
    respondWith(WITH_REMOTE)
    render(
      <DeleteBranchDialog
        repoPath="C:\\repo"
        name="feat/x"
        capabilities={{ ...ALL_CAPABILITIES, remotes: false }}
        mergedPullRequests={[]}
        onCancel={() => {}}
        onConfirm={async () => {}}
      />,
    )
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(screen.queryByRole('checkbox', { name: /Also delete remote branch/ })).toBeNull()
    expect(probeCalls()).toHaveLength(0)
  })

  it('没有远程分支：不预取', async () => {
    respondWith(INFO)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(probeCalls()).toHaveLength(0)
  })
})
