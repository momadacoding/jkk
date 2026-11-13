// @vitest-environment jsdom
//
// 删 worktree 撞上 worktree_busy 后的占用进程面板：在两个删除弹框里接得上、
// 结束要点两下、结束后重查。
import { StrictMode } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { DeleteWorktreeDialog } from '@/components/DeleteWorktreeDialog'
import { ARM_TIMEOUT_MS } from '@/components/WorktreeHoldersPanel'
import { DeleteBranchDialog, type DeleteInfo } from '@/components/DeleteBranchDialog'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import type { WorktreeHolder } from '@/engine/worktreeHolders'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const invokeMock = vi.mocked(invoke)

afterEach(() => {
  invokeMock.mockReset()
})

const WT_PATH = 'C:/repo/.claude/worktrees/wt'
const BUSY = { code: 'worktree_busy', message: '移不动这个 worktree：os error 32' }

const BROKER: WorktreeHolder = {
  pid: 15268,
  name: 'node.exe',
  kind: 'cwd',
  path: 'C:\\repo\\.claude\\worktrees\\wt',
  killable: true,
  isSelf: false,
  commandLine: null,
}

/** 按命令名分派的 invoke mock：每个命令一个按调用顺序取的答案队列。 */
function routeInvoke(routes: Record<string, unknown[]>) {
  invokeMock.mockImplementation(async (cmd: string) => {
    const queue = routes[cmd]
    if (!queue || queue.length === 0) throw new Error(`未预期的 invoke：${cmd}`)
    const next = queue.length > 1 ? queue.shift() : queue[0]
    if (next instanceof Error) throw next
    return next
  })
}

function callsOf(cmd: string) {
  return invokeMock.mock.calls.filter(([c]) => c === cmd)
}

function worktreeDialog(onConfirm: (force: boolean) => Promise<void>) {
  return (
    <DeleteWorktreeDialog
      repoPath="C:/repo"
      worktree={{ name: 'wt', path: WT_PATH, head: 'h', branch: 'feat/x', isMain: false }}
      onCancel={() => {}}
      onConfirm={onConfirm}
    />
  )
}

describe('WorktreeHoldersPanel（经由 DeleteWorktreeDialog）', () => {
  it('撞上 worktree_busy：列出进程；结束要点两下；结束后重查，空了提示重试', async () => {
    routeInvoke({
      worktree_holders: [[BROKER], []],
      terminate_worktree_holder: [undefined],
    })
    render(worktreeDialog(async () => { throw BUSY }))

    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    expect(await screen.findByText('node.exe')).toBeTruthy()
    expect(screen.getByText('PID 15268')).toBeTruthy()
    expect(screen.getByText(/工作目录停在 worktree 根目录/)).toBeTruthy()
    expect(callsOf('worktree_holders')[0][1]).toEqual({ path: 'C:/repo', worktreePath: WT_PATH })

    // 第一下只是待确认，不能真的结束
    fireEvent.click(screen.getByRole('button', { name: '结束进程' }))
    expect(callsOf('terminate_worktree_holder')).toHaveLength(0)

    fireEvent.click(screen.getByRole('button', { name: '确认结束' }))
    await waitFor(() => expect(callsOf('terminate_worktree_holder')).toHaveLength(1))
    expect(callsOf('terminate_worktree_holder')[0][1]).toEqual({
      path: 'C:/repo',
      worktreePath: WT_PATH,
      pid: 15268,
      heldPath: BROKER.path,
    })
    expect(await screen.findByText(/都结束了/)).toBeTruthy()
    expect(screen.queryByText('node.exe')).toBeNull()
  })

  it('StrictMode 下（dev 构建的真实形态）也能显示查到的进程', async () => {
    // 实测 bug：StrictMode 会把 effect 挂载→卸载→再挂载，卸载时把「还活着」标成 false、
    // 再挂载没改回来，查回来的结果全被丢掉，面板永远停在「正在查…」
    routeInvoke({ worktree_holders: [[BROKER]] })
    render(<StrictMode>{worktreeDialog(async () => { throw BUSY })}</StrictMode>)

    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    expect(await screen.findByText('node.exe')).toBeTruthy()
  })

  it('命令行一档把命令行摆出来，给用户看依据', async () => {
    const cmdline = 'node.exe broker.mjs serve --cwd C:/repo/.claude/worktrees/wt'
    routeInvoke({ worktree_holders: [[{ ...BROKER, kind: 'commandLine', commandLine: cmdline }]] })
    render(worktreeDialog(async () => { throw BUSY }))

    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    expect(await screen.findByText(cmdline)).toBeTruthy()
    expect(screen.getByText(/可能是它，也可能无关/)).toBeTruthy()
  })

  it('一个都没查到：说明多半是提权进程，而不是一片空白', async () => {
    routeInvoke({ worktree_holders: [[]] })
    render(worktreeDialog(async () => { throw BUSY }))

    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    expect(await screen.findByText(/管理员身份/)).toBeTruthy()
  })

  it('「确认结束」几秒不点就复原，走开回来一下点击不会直接结束', async () => {
    routeInvoke({ worktree_holders: [[BROKER]] })
    render(worktreeDialog(async () => { throw BUSY }))
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    const button = await screen.findByRole('button', { name: '结束进程' })

    vi.useFakeTimers()
    try {
      fireEvent.click(button)
      expect(screen.getByRole('button', { name: '确认结束' })).toBeTruthy()
      act(() => { vi.advanceTimersByTime(ARM_TIMEOUT_MS + 10) })
      expect(screen.getByRole('button', { name: '结束进程' })).toBeTruthy()
    } finally {
      vi.useRealTimers()
    }
    expect(callsOf('terminate_worktree_holder')).toHaveLength(0)
  })

  it('正在结束进程时 Delete 禁用，结束完放开', async () => {
    let release!: () => void
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'worktree_holders') return [BROKER]
      return new Promise<void>((r) => { release = r })
    })
    render(worktreeDialog(async () => { throw BUSY }))
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    fireEvent.click(await screen.findByRole('button', { name: '结束进程' }))
    fireEvent.click(screen.getByRole('button', { name: '确认结束' }))

    const del = screen.getByRole('button', { name: 'Delete' }) as HTMLButtonElement
    await waitFor(() => expect(del.disabled).toBe(true))
    act(() => release())
    await waitFor(() => expect(del.disabled).toBe(false))
  })

  it('不能结束的进程照样列出，但没有按钮，并说明原因', async () => {
    routeInvoke({ worktree_holders: [[{ ...BROKER, name: 'jkk.exe', killable: false, isSelf: true }]] })
    render(worktreeDialog(async () => { throw BUSY }))

    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    expect(await screen.findByText('jkk.exe')).toBeTruthy()
    // cwd 一档的「自己」：多半是从停在 worktree 里的终端启动了本应用，要从别处重启
    expect(screen.getByText(/本应用或它的子进程/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: '结束进程' })).toBeNull()
  })

  it('结束失败：显示后端的原因，并重查列表', async () => {
    // 后端失败抛的是 { code, message } 普通对象，不是 Error
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'worktree_holders') return [BROKER]
      throw { code: 'terminate_failed', message: '打不开进程 15268' }
    })
    render(worktreeDialog(async () => { throw BUSY }))

    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    fireEvent.click(await screen.findByRole('button', { name: '结束进程' }))
    fireEvent.click(screen.getByRole('button', { name: '确认结束' }))

    expect(await screen.findByText(/打不开进程 15268/)).toBeTruthy()
    await waitFor(() => expect(callsOf('worktree_holders')).toHaveLength(2))
  })

  it('别的失败（如有未提交改动）不查占用进程', async () => {
    routeInvoke({})
    render(worktreeDialog(async () => { throw { code: 'worktree_dirty', message: 'dirty' } }))

    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    expect(await screen.findByText(/dirty/)).toBeTruthy()
    expect(callsOf('worktree_holders')).toHaveLength(0)
  })

  it('再点删除又撞上：面板重建、重新查一遍', async () => {
    routeInvoke({ worktree_holders: [[BROKER]] })
    render(worktreeDialog(async () => { throw BUSY }))

    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    expect(await screen.findByText('node.exe')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    await waitFor(() => expect(callsOf('worktree_holders')).toHaveLength(2))
  })
})

describe('WorktreeHoldersPanel（经由 DeleteBranchDialog）', () => {
  it('勾着「同时删 worktree」、删 worktree 那步撞上 worktree_busy：按那个 worktree 查', async () => {
    const info: DeleteInfo = {
      upstream: null,
      remote: null,
      remoteBranch: null,
      remoteProtected: false,
      unmerged: 0,
      comparedTo: 'main',
      contentMerged: false,
      mergedVia: null,
      mergedInto: null,
      worktree: { name: 'wt', path: WT_PATH },
    }
    routeInvoke({ branch_delete_info: [info], worktree_holders: [[BROKER]] })
    render(
      <DeleteBranchDialog
        repoPath="C:/repo"
        name="feat/x"
        capabilities={ALL_CAPABILITIES}
        mergedPullRequests={[]}
        onCancel={() => {}}
        onConfirm={async () => { throw BUSY }}
      />,
    )

    const del = screen.getByRole('button', { name: 'Delete' }) as HTMLButtonElement
    await waitFor(() => expect(del.disabled).toBe(false))
    fireEvent.click(del)

    expect(await screen.findByText('node.exe')).toBeTruthy()
    expect(callsOf('worktree_holders')[0][1]).toEqual({ path: 'C:/repo', worktreePath: WT_PATH })
  })
})
