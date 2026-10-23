// @vitest-environment jsdom
//
// refreshDiff 的写回规则（Codex review 第 2 条）：用户操作的回调会在切换视图之后才发出 `workdir` 刷新，
// 它不能让当前提交的 diff 作废，也不能盖掉当前提交的 diff。
import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { DiffData } from '@/engine/types'

interface Pending {
  cmd: string
  args: Record<string, unknown>
  resolve: (value: unknown) => void
  reject: (reason: unknown) => void
}

const pending: Pending[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: Record<string, unknown>) =>
    new Promise((resolve, reject) => { pending.push({ cmd, args, resolve, reject }) }),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useRepoStore } = await import('@/store/repoStore')

function diffOf(path: string): DiffData {
  return { files: [{ path, status: 'modified', oldPath: null, binary: false, hunks: [] }] }
}

/** 找到某个 scope 的那次 read_diff 并让它返回。 */
async function answer(scope: string, data: DiffData) {
  const call = pending.find((p) => p.cmd === 'read_diff' && p.args.scope === scope)
  if (!call) throw new Error(`没有 scope 为 ${scope} 的请求`)
  pending.splice(pending.indexOf(call), 1)
  await act(async () => { call.resolve(data) })
}

beforeEach(() => { pending.length = 0 })

describe('refreshDiff 只写回当前视图要的那一份', () => {
  it('切到提交 B 之后才发出的 workdir 刷新，不能让 B 的结果作废（workdir 先回）', async () => {
    const { result } = renderHook(() => useRepoStore())
    result.current.wantedDiffScope.current = 'commit:B'
    let b!: Promise<void>
    let w!: Promise<void>
    act(() => {
      b = result.current.refreshDiff('C:/r', 'commit:B')
      w = result.current.refreshDiff('C:/r', 'workdir')
    })
    await answer('workdir', diffOf('dirty.txt'))
    await answer('commit:B', diffOf('b.txt'))
    await act(async () => { await Promise.all([b, w]) })
    expect(result.current.state.diff.files.map((f) => f.path)).toEqual(['b.txt'])
  })

  it('同上，B 先回、workdir 后回也不能盖掉 B', async () => {
    const { result } = renderHook(() => useRepoStore())
    result.current.wantedDiffScope.current = 'commit:B'
    act(() => {
      void result.current.refreshDiff('C:/r', 'commit:B')
      void result.current.refreshDiff('C:/r', 'workdir')
    })
    await answer('commit:B', diffOf('b.txt'))
    await answer('workdir', diffOf('dirty.txt'))
    expect(result.current.state.diff.files.map((f) => f.path)).toEqual(['b.txt'])
  })

  it('同一个 scope 发了两次：先发的晚到时丢掉，只留后发的', async () => {
    const { result } = renderHook(() => useRepoStore())
    result.current.wantedDiffScope.current = 'workdir'
    act(() => {
      void result.current.refreshDiff('C:/r', 'workdir')
      void result.current.refreshDiff('C:/r', 'workdir')
    })
    const [older, newer] = pending.filter((p) => p.cmd === 'read_diff')
    await act(async () => { newer.resolve(diffOf('new.txt')) })
    await act(async () => { older.resolve(diffOf('old.txt')) })
    expect(result.current.state.diff.files.map((f) => f.path)).toEqual(['new.txt'])
  })

  it('还不知道当前视图要哪一份时（刚打开仓库）照常写回', async () => {
    const { result } = renderHook(() => useRepoStore())
    act(() => { void result.current.refreshDiff('C:/r', 'workdir') })
    await answer('workdir', diffOf('a.txt'))
    expect(result.current.state.diff.files.map((f) => f.path)).toEqual(['a.txt'])
  })
})

// 第 65 条：Stage / Unstage 之后选中的文件可能换了区，刷新 diff 要按**新的**状态算读哪个区，
// 所以 refreshStatus 把它读到的状态交回去；被更晚的请求盖掉时交回 undefined，调用方别拿它算
describe('refreshStatus 交回读到的状态', () => {
  const status = { unstaged: [], staged: [{ path: 'a.txt', status: 'modified', additions: 1, deletions: 0 }] }

  it('最新的一次：交回读到的状态', async () => {
    const { result } = renderHook(() => useRepoStore())
    let run!: Promise<unknown>
    act(() => { run = result.current.refreshStatus('C:/r') })
    const call = pending.find((p) => p.cmd === 'read_status')!
    await act(async () => { call.resolve(status) })
    await expect(run).resolves.toEqual(status)
  })

  it('被后发的请求盖掉：交回 undefined', async () => {
    const { result } = renderHook(() => useRepoStore())
    let older!: Promise<unknown>
    act(() => {
      older = result.current.refreshStatus('C:/r')
      void result.current.refreshStatus('C:/r')
    })
    const [first] = pending.filter((p) => p.cmd === 'read_status')
    await act(async () => { first.resolve(status) })
    await expect(older).resolves.toBeUndefined()
  })
})

// 写操作（Stage / Unstage / 丢弃 / 提交）之后刷新「本地改动」：按刷新后的状态算读哪个区
describe('refreshChanges', () => {
  const ch = (path: string) => ({ path, status: 'modified', additions: 1, deletions: 0 })
  const unstagedA = () => ({ section: 'unstaged' as const, paths: ['a.txt'], anchor: 'a.txt' })

  it('选中的文件刚被整份暂存：读 Staged 的 diff，不白读一整份 Unstaged', async () => {
    const { result } = renderHook(() => useRepoStore())
    act(() => { void result.current.refreshChanges('C:/r', unstagedA) })
    const status = pending.find((p) => p.cmd === 'read_status')!
    await act(async () => { status.resolve({ unstaged: [ch('b.txt')], staged: [ch('a.txt')] }) })
    expect(pending.filter((p) => p.cmd === 'read_diff').map((p) => p.args.scope)).toEqual(['staged'])
  })

  // 盖掉它的那次状态刷新可能来自工具栏、checkout 等，它们不读 diff；这里跳过的话右侧就停在旧的
  it('状态被更晚的刷新盖掉：照样读 diff，按 store 里最新的状态算区', async () => {
    const { result } = renderHook(() => useRepoStore())
    act(() => {
      void result.current.refreshChanges('C:/r', unstagedA)
      void result.current.refreshStatus('C:/r')
    })
    const [older, newer] = pending.filter((p) => p.cmd === 'read_status')
    await act(async () => { newer.resolve({ unstaged: [ch('b.txt')], staged: [ch('a.txt')] }) })
    await act(async () => { older.resolve({ unstaged: [ch('a.txt')], staged: [] }) })
    expect(pending.filter((p) => p.cmd === 'read_diff').map((p) => p.args.scope)).toEqual(['staged'])
  })
})

// 界面用 `currentDiff(diff, diffScope, 当前视图要的)` 决定显示 diff 还是「正在读取」
describe('refreshDiff 记下 diff 是哪个 scope 的', () => {
  it('写回时一起记下 scope；新请求回来之前 diffScope 仍是上一份的', async () => {
    const { result } = renderHook(() => useRepoStore())
    expect(result.current.state.diffScope).toBeNull()
    result.current.wantedDiffScope.current = 'workdir'
    act(() => { void result.current.refreshDiff('C:/r', 'workdir') })
    await answer('workdir', diffOf('dirty.txt'))
    expect(result.current.state.diffScope).toBe('workdir')

    result.current.wantedDiffScope.current = 'commit:B'
    act(() => { void result.current.refreshDiff('C:/r', 'commit:B') })
    // 大提交的 diff 还在读：store 里仍是工作区那份，scope 也如实是 workdir
    expect(result.current.state.diffScope).toBe('workdir')
    await answer('commit:B', diffOf('b.txt'))
    expect(result.current.state.diffScope).toBe('commit:B')
  })

  it('读失败：记成这个 scope 的空 diff（不让界面一直说正在读取），错误照旧抛给调用方', async () => {
    const { result } = renderHook(() => useRepoStore())
    result.current.wantedDiffScope.current = 'commit:B'
    let run!: Promise<void>
    act(() => { run = result.current.refreshDiff('C:/r', 'commit:B') })
    const call = pending.find((p) => p.cmd === 'read_diff')!
    await act(async () => {
      call.reject(new Error('boom'))
      await expect(run).rejects.toThrow('boom')
    })
    expect(result.current.state.diffScope).toBe('commit:B')
    expect(result.current.state.diff.files).toEqual([])
  })
})
