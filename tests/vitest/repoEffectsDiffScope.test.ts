// @vitest-environment jsdom
//
// 第 65 条：「本地改动」视图按选中文件所在的区读 diff，不再固定读 HEAD 对比工作区（`workdir`）。
import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const calls: { cmd: string, args: Record<string, unknown> }[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: Record<string, unknown>) => {
    calls.push({ cmd, args })
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

function mount(viewMode: string, changesScope: 'unstaged' | 'staged') {
  return renderHook(
    ({ viewMode, changesScope }) => {
      const store = useRepoStore()
      const state = { ...store.state, path: 'C:/r' }
      useRepoEffects(state, store, viewMode, changesScope)
      return store
    },
    { initialProps: { viewMode, changesScope } },
  )
}

describe('useRepoEffects：本地改动按区读 diff', () => {
  it('选中的在 Staged：读 staged，不读 workdir', () => {
    const { result } = mount('changes', 'staged')
    expect(diffScopes()).toEqual(['staged'])
    expect(result.current.wantedDiffScope.current).toBe('staged')
  })

  it('选中的在 Unstaged：读 unstaged', () => {
    mount('changes', 'unstaged')
    expect(diffScopes()).toEqual(['unstaged'])
  })

  it('换了区就重读那个区的', () => {
    const { rerender, result } = mount('changes', 'unstaged')
    rerender({ viewMode: 'changes', changesScope: 'staged' })
    expect(diffScopes()).toEqual(['unstaged', 'staged'])
    expect(result.current.wantedDiffScope.current).toBe('staged')
  })

  // 监听器的 effect 依赖 diffScope，换区就重新注册一次。`listen` 是一次 IPC 往返：
  // 旧的注册还没回来就被清理的话，回来之后得立刻注销，否则每换一次区就多挂一个监听器，
  // 之后每次存盘都多跑一整套刷新（代码 review 第 1 条）
  it('换区快过 IPC 往返：被清理掉的那次注册回来之后立刻注销，留下的只有一个', async () => {
    const { rerender } = mount('changes', 'unstaged')
    rerender({ viewMode: 'changes', changesScope: 'staged' })
    const repoChanged = registrations.filter((r) => r.event === 'repo-changed')
    expect(repoChanged).toHaveLength(2)
    await act(async () => { for (const r of repoChanged) r.done() })
    expect(repoChanged[0].unlisten).toHaveBeenCalledTimes(1)
    expect(repoChanged[1].unlisten).not.toHaveBeenCalled()
  })

  // 文件监听在 Stage 之后触发时，选中的文件可能已经换了区：按事件到达时当前视图要的 scope 读，
  // 不按注册监听那一刻的（那一份读回来只会被丢掉）
  it('文件变更事件按事件到达时的 scope 读 diff，不按注册时的', () => {
    const { result } = mount('changes', 'unstaged')
    const [reg] = registrations.filter((r) => r.event === 'repo-changed')
    calls.length = 0
    result.current.wantedDiffScope.current = 'staged'
    reg.handler({ payload: 'C:/r' })
    expect(diffScopes()).toEqual(['staged'])
  })

  it('在提交图视图里：区与读哪份 diff 无关', () => {
    mount('graph', 'staged')
    expect(diffScopes()).not.toContain('staged')
    expect(diffScopes()).not.toContain('unstaged')
  })
})
