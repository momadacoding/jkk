// @vitest-environment jsdom
//
// 组件测试只在这个文件里切到 jsdom，其余纯逻辑测试仍跑在 node 环境（vitest.config.ts）。
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { DeleteBranchDialog, type DeleteInfo } from '@/components/DeleteBranchDialog'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
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
})

describe('DeleteBranchDialog 预取远程默认分支', () => {
  const WITH_REMOTE: DeleteInfo = {
    ...INFO,
    upstream: 'origin/feat/x',
    remote: 'origin',
    remoteBranch: 'feat/x',
  }

  /** 按命令名分派：`branch_delete_info` 回 `info`，其余（预取）回 undefined。 */
  function respondWith(info: DeleteInfo) {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'branch_delete_info' ? info : undefined)
  }

  function prefetchCalls() {
    return invokeMock.mock.calls.filter(([cmd]) => cmd === 'prefetch_remote_default')
  }

  function remoteCheckbox(): HTMLInputElement {
    return screen.getByRole('checkbox', { name: /Also delete remote branch/ }) as HTMLInputElement
  }

  it('只删本地（远程那项没勾）：不走网络——SSH agent 审批、凭据框都不该为此弹出来', async () => {
    respondWith(WITH_REMOTE)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(remoteCheckbox().checked).toBe(false)
    expect(prefetchCalls()).toHaveLength(0)
  })

  it('勾上远程那项就预取，护栏在点确认时不必再握一次手', async () => {
    respondWith(WITH_REMOTE)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(prefetchCalls()).toEqual([
      // JSX 属性字符串不处理转义：dialog() 里的 repoPath="C:\\repo" 实际是两个反斜杠
      ['prefetch_remote_default', { path: String.raw`C:\\repo`, remote: 'origin' }],
    ]))
  })

  it('PR 列表晚到触发重查：远程没变，不再发一趟', async () => {
    respondWith(WITH_REMOTE)
    const { rerender } = render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(prefetchCalls()).toHaveLength(1))
    rerender(dialog({ prs: [MERGED_PR] }))
    await waitFor(() =>
      expect(invokeMock.mock.calls.filter(([c]) => c === 'branch_delete_info')).toHaveLength(2))
    expect(prefetchCalls()).toHaveLength(1)
  })

  it('勾着的时候重查发现远程分支受保护：换了远程也不预取（门控看的是能不能删，不只是勾没勾）', async () => {
    let current = WITH_REMOTE
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'branch_delete_info' ? current : undefined)
    const { rerender } = render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(prefetchCalls()).toHaveLength(1))

    current = { ...WITH_REMOTE, remote: 'upstream', remoteProtected: true }
    rerender(dialog({ prs: [MERGED_PR] }))
    await waitFor(() => expect(remoteCheckbox().disabled).toBe(true))
    expect(prefetchCalls()).toHaveLength(1)
  })

  it('取消再重新勾选会再发一次：上次过期或失败时靠这个刷新，去重在后端', async () => {
    respondWith(WITH_REMOTE)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    fireEvent.click(remoteCheckbox())
    fireEvent.click(remoteCheckbox())
    fireEvent.click(remoteCheckbox())
    await waitFor(() => expect(prefetchCalls()).toHaveLength(2))
  })

  it('默认分支的勾选框是禁用的，点不上，也就不会预取', async () => {
    respondWith({ ...WITH_REMOTE, remoteProtected: true })
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(remoteCheckbox().disabled).toBe(true)
    fireEvent.click(remoteCheckbox())
    expect(remoteCheckbox().checked).toBe(false)
    expect(prefetchCalls()).toHaveLength(0)
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
    expect(prefetchCalls()).toHaveLength(0)
  })

  it('没有远程分支：不预取', async () => {
    respondWith(INFO)
    render(dialog({ prs: [] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(prefetchCalls()).toHaveLength(0)
  })
})
