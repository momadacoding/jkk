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

  // 2026-10-30 排查：并发的两轮抢同一块磁盘，而且先发的那轮做完被丢掉，界面要等第二轮
  it('同一个 scope 在读时再来：不并发；先读完的照常写回，再补读一次', async () => {
    const { result } = renderHook(() => useRepoStore())
    result.current.wantedDiffScope.current = 'workdir'
    act(() => {
      void result.current.refreshDiff('C:/r', 'workdir')
      void result.current.refreshDiff('C:/r', 'workdir')
    })
    expect(pending.filter((p) => p.cmd === 'read_diff')).toHaveLength(1)
    await answer('workdir', diffOf('first.txt'))
    expect(result.current.state.diff.files.map((f) => f.path)).toEqual(['first.txt'])
    expect(pending.filter((p) => p.cmd === 'read_diff')).toHaveLength(1)
    await answer('workdir', diffOf('trailing.txt'))
    expect(result.current.state.diff.files.map((f) => f.path)).toEqual(['trailing.txt'])
  })

  it('换了仓库：旧仓库同一个 scope 晚到的 diff 不盖掉新仓库的', async () => {
    const { result } = renderHook(() => useRepoStore())
    result.current.wantedDiffScope.current = 'workdir'
    act(() => {
      void result.current.refreshDiff('C:/a', 'workdir')
      void result.current.refreshDiff('C:/b', 'workdir')
    })
    const reads = pending.filter((p) => p.cmd === 'read_diff')
    expect(reads.map((p) => p.args.path)).toEqual(['C:/a', 'C:/b'])
    await act(async () => { reads[1].resolve(diffOf('b.txt')) })
    await act(async () => { reads[0].resolve(diffOf('a.txt')) })
    expect(result.current.state.diff.files.map((f) => f.path)).toEqual(['b.txt'])
  })

  it('还不知道当前视图要哪一份时（刚打开仓库）照常写回', async () => {
    const { result } = renderHook(() => useRepoStore())
    act(() => { void result.current.refreshDiff('C:/r', 'workdir') })
    await answer('workdir', diffOf('a.txt'))
    expect(result.current.state.diff.files.map((f) => f.path)).toEqual(['a.txt'])
  })
})

// 第 65 条：Stage / Unstage 之后选中的文件可能换了区，刷新 diff 要按**新的**状态算读哪个区，
// 所以 refreshStatus 把它读到的状态交回去
describe('refreshStatus 交回读到的状态', () => {
  const status = { unstaged: [], staged: [{ path: 'a.txt', status: 'modified', additions: 1, deletions: 0 }] }
  const later = { unstaged: [], staged: [{ path: 'b.txt', status: 'modified', additions: 1, deletions: 0 }] }

  it('交回读到的状态', async () => {
    const { result } = renderHook(() => useRepoStore())
    let run!: Promise<unknown>
    act(() => { run = result.current.refreshStatus('C:/r') })
    const call = pending.find((p) => p.cmd === 'read_status')!
    await act(async () => { call.resolve(status) })
    await expect(run).resolves.toEqual(status)
  })

  // 2026-10-30 排查的主因：Stage 的回调先发一轮，约 300ms 后文件监听（`git add` 改了 index）
  // 再发一轮。原来按「只认最后一次」，先发的那轮做完被丢掉，界面要等第二轮，约 5 秒
  it('在读时再来：不并发；先发的那轮照常写回并交回，后来的拿补读那一轮的结果', async () => {
    const { result } = renderHook(() => useRepoStore())
    let first!: Promise<unknown>
    let second!: Promise<unknown>
    act(() => {
      first = result.current.refreshStatus('C:/r')
      second = result.current.refreshStatus('C:/r')
    })
    expect(pending.filter((p) => p.cmd === 'read_status')).toHaveLength(1)
    const own = pending.find((p) => p.cmd === 'read_status')!
    pending.splice(pending.indexOf(own), 1)
    await act(async () => { own.resolve(status) })
    await expect(first).resolves.toEqual(status)
    expect(result.current.state.status).toEqual(status)

    const trailing = pending.filter((p) => p.cmd === 'read_status')
    expect(trailing).toHaveLength(1)
    pending.splice(pending.indexOf(trailing[0]), 1)
    await act(async () => { trailing[0].resolve(later) })
    await expect(second).resolves.toEqual(later)
    expect(result.current.state.status).toEqual(later)
  })

  // 现在每个标签页的路径是固定的，但 store 不该依赖这一点（独立 review）
  it('换了仓库：新仓库的读取不排在旧仓库后面，旧仓库晚到的结果不盖掉新仓库', async () => {
    const { result } = renderHook(() => useRepoStore())
    let b!: Promise<unknown>
    act(() => {
      void result.current.refreshStatus('C:/a')
      b = result.current.refreshStatus('C:/b')
    })
    const reads = pending.filter((p) => p.cmd === 'read_status')
    expect(reads.map((p) => p.args.path)).toEqual(['C:/a', 'C:/b'])
    for (const p of reads) pending.splice(pending.indexOf(p), 1)
    await act(async () => { reads[1].resolve(later) })
    await expect(b).resolves.toEqual(later)
    await act(async () => { reads[0].resolve(status) })
    expect(result.current.state.status).toEqual(later)
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

  // Stage 的回调先读状态，文件监听那一轮在它读的过程中到达：不并发，也不让它作废
  it('状态读取进行中又来一次刷新：不等补读那一轮，用自己那轮的结果算区读 diff', async () => {
    const { result } = renderHook(() => useRepoStore())
    act(() => {
      void result.current.refreshChanges('C:/r', unstagedA)
      void result.current.refreshStatus('C:/r')
    })
    expect(pending.filter((p) => p.cmd === 'read_status')).toHaveLength(1)
    const own = pending.find((p) => p.cmd === 'read_status')!
    pending.splice(pending.indexOf(own), 1)
    await act(async () => { own.resolve({ unstaged: [ch('b.txt')], staged: [ch('a.txt')] }) })
    expect(pending.filter((p) => p.cmd === 'read_diff').map((p) => p.args.scope)).toEqual(['staged'])
  })

  // 在读的那一轮可能是写操作之前发起的（比如上一次保存触发的监听），它读到的是旧状态
  it('写操作之前就已经在读的状态不算数：等补读那一轮的结果再算区', async () => {
    const { result } = renderHook(() => useRepoStore())
    act(() => { void result.current.refreshStatus('C:/r') })
    // 这时用户点了 Stage，回调随后刷新
    act(() => { void result.current.refreshChanges('C:/r', unstagedA) })
    const stale = pending.find((p) => p.cmd === 'read_status')!
    pending.splice(pending.indexOf(stale), 1)
    await act(async () => { stale.resolve({ unstaged: [ch('a.txt')], staged: [] }) })
    expect(pending.filter((p) => p.cmd === 'read_diff')).toEqual([])
    const fresh = pending.find((p) => p.cmd === 'read_status')!
    pending.splice(pending.indexOf(fresh), 1)
    await act(async () => { fresh.resolve({ unstaged: [ch('b.txt')], staged: [ch('a.txt')] }) })
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
