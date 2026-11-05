// @vitest-environment jsdom
//
// 第 65 条：「本地改动」视图按选中文件所在的区读 diff，不再固定读 HEAD 对比工作区（`workdir`）。
// 6.13 方案 C 起只读选中的那一个文件：scope 是 `<区>:<路径>`。
import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const calls: { cmd: string, args: Record<string, unknown> }[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: Record<string, unknown>) => {
    calls.push({ cmd, args })
    // diff 立即读完：单文件的 diff 排在同一队里（6.13 方案 C），永不返回的话换选中之后的那次
    // 只会排着队、不发出去。其余的读取一直挂着，这里不关心它们的结果
    if (cmd === 'read_diff') return Promise.resolve({ files: [] })
    return new Promise(() => {})
  },
}))
/** 每次 `listen` 的注册：IPC 往返由测试决定什么时候完成。 */
interface Registration {
  event: string
  handler: (e: { payload: string }) => void
  unlisten: ReturnType<typeof vi.fn>
  done: () => void
}
const registrations: Registration[] = []

vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: string }) => void) => new Promise((resolve) => {
    const unlisten = vi.fn()
    registrations.push({ event, handler, unlisten, done: () => resolve(unlisten) })
  }),
}))

const { useRepoEffects, useRepoStore } = await import('@/store/repoStore')

beforeEach(() => {
  calls.length = 0
  registrations.length = 0
})

const diffScopes = () => calls.filter((c) => c.cmd === 'read_diff').map((c) => c.args.scope)

function mount(viewMode: string, changesScope: string | null, path = 'C:/r') {
  return renderHook(
    ({ viewMode, changesScope, path }) => {
      const store = useRepoStore()
      const state = { ...store.state, path }
      useRepoEffects(state, store, viewMode, changesScope, true)
      return store
    },
    { initialProps: { viewMode, changesScope, path } },
  )
}

describe('useRepoEffects：本地改动按区读 diff', () => {
  it('选中的在 Staged：读 staged，不读 workdir', () => {
    const { result } = mount('changes', 'staged:s.txt')
    expect(diffScopes()).toEqual(['staged:s.txt'])
    expect(result.current.wantedDiffScope.current).toBe('staged:s.txt')
  })

  it('选中的在 Unstaged：读 unstaged', () => {
    mount('changes', 'unstaged:u.txt')
    expect(diffScopes()).toEqual(['unstaged:u.txt'])
  })

  it('换了区就重读那个区的', async () => {
    const { rerender, result } = mount('changes', 'unstaged:u.txt')
    await act(async () => {})
    rerender({ viewMode: 'changes', changesScope: 'staged:s.txt', path: 'C:/r' })
    expect(diffScopes()).toEqual(['unstaged:u.txt', 'staged:s.txt'])
    expect(result.current.wantedDiffScope.current).toBe('staged:s.txt')
  })

  // 6.13 方案 C 起 scope 随选中的文件变，方向键每按一下就变一次。监听若跟着重新注册，
  // 注销与重新注册之间那一次 IPC 往返里的 `repo-changed` 会丢（独立 review 第 4 条）。
  // 处理函数按事件到达时的 `wantedDiffScope` 读，本来就不需要跟着 scope 重新注册
  it('换选中的文件、换区：不重新注册监听', async () => {
    const { rerender } = mount('changes', 'unstaged:u.txt')
    await act(async () => {})
    rerender({ viewMode: 'changes', changesScope: 'unstaged:v.txt', path: 'C:/r' })
    rerender({ viewMode: 'changes', changesScope: 'staged:s.txt', path: 'C:/r' })
    expect(registrations.filter((r) => r.event === 'repo-changed')).toHaveLength(1)
  })

  // 换仓库时重新注册。`listen` 是一次 IPC 往返：旧的注册还没回来就被清理的话，回来之后得立刻注销，
  // 否则每换一次就多挂一个监听器，之后每次存盘都多跑一整套刷新（代码 review 第 1 条）
  it('换仓库快过 IPC 往返：被清理掉的那次注册回来之后立刻注销，留下的只有一个', async () => {
    const { rerender } = mount('changes', 'unstaged:u.txt')
    rerender({ viewMode: 'changes', changesScope: 'unstaged:u.txt', path: 'C:/other' })
    const repoChanged = registrations.filter((r) => r.event === 'repo-changed')
    expect(repoChanged).toHaveLength(2)
    await act(async () => { for (const r of repoChanged) r.done() })
    expect(repoChanged[0].unlisten).toHaveBeenCalledTimes(1)
    expect(repoChanged[1].unlisten).not.toHaveBeenCalled()
  })

  // 文件监听在 Stage 之后触发时，选中的文件可能已经换了区：按事件到达时当前视图要的 scope 读，
  // 不按注册监听那一刻的（那一份读回来只会被丢掉）
  it('文件变更事件按事件到达时的 scope 读 diff，不按注册时的', async () => {
    const { result } = mount('changes', 'unstaged:u.txt')
    await act(async () => {})
    const [reg] = registrations.filter((r) => r.event === 'repo-changed')
    calls.length = 0
    result.current.wantedDiffScope.current = 'staged:s.txt'
    reg.handler({ payload: 'C:/r' })
    expect(diffScopes()).toEqual(['staged:s.txt'])
  })

  // 两边都没有文件：右侧没东西可显示。原来照样读整个 Unstaged，等于白扫一遍工作区（6.13 方案 C）
  it('本地改动里没有文件可选：不读 diff', () => {
    const { result } = mount('changes', null)
    expect(diffScopes()).toEqual([])
    expect(result.current.wantedDiffScope.current).toBeNull()
  })

  it('在提交图视图里：区与读哪份 diff 无关', () => {
    mount('graph', 'staged:s.txt')
    expect(diffScopes()).not.toContain('staged:s.txt')
    expect(diffScopes()).not.toContain('unstaged:u.txt')
  })
})
