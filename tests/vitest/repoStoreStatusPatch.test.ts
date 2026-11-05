// @vitest-environment jsdom
//
// 2026-11-05 排查报告 6.13 方案 A：Stage / Unstage 之后只重读刚操作的那几个路径（`read_status_paths`），
// 不再把整个工作区扫一遍（真实工作副本上一次 1.35–2.33 秒）。
import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { FileChange, StatusData } from '@/engine/types'
import type { StatusPatch } from '@/engine/statusPatch'

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

const m = (path: string): FileChange => ({ path, status: 'modified', additions: 1, deletions: 0 })
const statusReads = () => pending.filter((p) => p.cmd === 'read_status' || p.cmd === 'read_status_paths')

/** 取出最早的一次状态读取（全量或局部），让它返回。 */
async function answer(cmd: 'read_status' | 'read_status_paths', value: StatusData | StatusPatch) {
  const call = pending.find((p) => p.cmd === cmd)
  if (!call) throw new Error(`没有 ${cmd} 请求；在等的是 ${pending.map((p) => p.cmd).join(', ')}`)
  pending.splice(pending.indexOf(call), 1)
  await act(async () => { call.resolve(value) })
}

async function fail(cmd: string) {
  const call = pending.find((p) => p.cmd === cmd)!
  pending.splice(pending.indexOf(call), 1)
  await act(async () => { call.reject({ code: 'boom', message: 'boom' }) })
}

const unstagedA = () => ({ section: 'unstaged' as const, paths: ['a.txt'], anchor: 'a.txt' })
const initial: StatusData = { staged: [], unstaged: [m('a.txt'), m('b.txt')] }

/** 先全量读一次，store 里有了「上一次的状态」。 */
async function loaded() {
  const hook = renderHook(() => useRepoStore())
  act(() => { void hook.result.current.refreshStatus('C:/r') })
  await answer('read_status', initial)
  return hook
}

beforeEach(() => { pending.length = 0 })

describe('Stage / Unstage 之后的局部重读', () => {
  it('只读刚 Stage 的路径，就地更新列表，不做全量读', async () => {
    const { result } = await loaded()
    act(() => { void result.current.refreshChanges('C:/r', unstagedA, ['a.txt']) })
    expect(statusReads().map((p) => [p.cmd, p.args.paths])).toEqual([['read_status_paths', ['a.txt']]])
    await answer('read_status_paths', { staged: [m('a.txt')], unstaged: [], ignoreCase: true })
    expect(result.current.state.status).toEqual({ staged: [m('a.txt')], unstaged: [m('b.txt')] })
    expect(statusReads()).toEqual([])
  })

  it('还没读过状态（不知道「上一次」是什么）：照常全量读', async () => {
    const { result } = renderHook(() => useRepoStore())
    act(() => { void result.current.refreshChanges('C:/r', unstagedA, ['a.txt']) })
    expect(statusReads().map((p) => p.cmd)).toEqual(['read_status'])
  })

  it('不带路径的写操作（丢弃、提交）照常全量读', async () => {
    const { result } = await loaded()
    act(() => { void result.current.refreshChanges('C:/r', unstagedA) })
    expect(statusReads().map((p) => p.cmd)).toEqual(['read_status'])
  })

  it('git 的连带改动（没点名的路径在 Staged 里也变了）：退回全量读', async () => {
    const { result } = await loaded()
    act(() => { void result.current.refreshChanges('C:/r', unstagedA, ['a.txt']) })
    await answer('read_status_paths', { staged: [m('a.txt'), m('x.txt')], unstaged: [], ignoreCase: true })
    expect(statusReads().map((p) => p.cmd)).toEqual(['read_status'])
    const truth: StatusData = { staged: [m('a.txt'), m('x.txt')], unstaged: [] }
    await answer('read_status', truth)
    expect(result.current.state.status).toEqual(truth)
  })

  it('局部重读失败：退回全量读，不让列表停在 Stage 之前', async () => {
    const { result } = await loaded()
    act(() => { void result.current.refreshChanges('C:/r', unstagedA, ['a.txt']) })
    await fail('read_status_paths')
    expect(statusReads().map((p) => p.cmd)).toEqual(['read_status'])
    const truth: StatusData = { staged: [m('a.txt')], unstaged: [m('b.txt')] }
    await answer('read_status', truth)
    expect(result.current.state.status).toEqual(truth)
  })

  // 全量读可能是 Stage 之前发起的（文件监听），它读到的是 Stage 之前的状态。局部重读排在它后面、
  // 在它的结果上合并，不能先合并、再被它晚到的旧结果盖掉
  it('全量读在跑时 Stage：局部重读排在它后面，在它的结果上合并', async () => {
    const { result } = await loaded()
    act(() => { void result.current.refreshStatus('C:/r') })
    act(() => { void result.current.refreshChanges('C:/r', unstagedA, ['a.txt']) })
    expect(statusReads().map((p) => p.cmd)).toEqual(['read_status'])
    await answer('read_status', { staged: [], unstaged: [m('a.txt'), m('b.txt'), m('c.txt')] })
    expect(statusReads().map((p) => p.cmd)).toEqual(['read_status_paths'])
    await answer('read_status_paths', { staged: [m('a.txt')], unstaged: [], ignoreCase: true })
    expect(result.current.state.status).toEqual({ staged: [m('a.txt')], unstaged: [m('b.txt'), m('c.txt')] })
  })

  // 排队期间又来了一次全量刷新（外部改动）：补跑那一轮必须是全量的，局部重读会漏掉外部改动
  it('排队的局部重读遇上排队的全量刷新：补跑那一轮做全量读', async () => {
    const { result } = await loaded()
    act(() => { void result.current.refreshStatus('C:/r') })
    act(() => {
      void result.current.refreshChanges('C:/r', unstagedA, ['a.txt'])
      void result.current.refreshStatus('C:/r')
    })
    await answer('read_status', initial)
    expect(statusReads().map((p) => p.cmd)).toEqual(['read_status'])
  })

  it('全量刷新排在前、局部重读排在后，补跑一样做全量读', async () => {
    const { result } = await loaded()
    act(() => { void result.current.refreshStatus('C:/r') })
    act(() => {
      void result.current.refreshStatus('C:/r')
      void result.current.refreshChanges('C:/r', unstagedA, ['a.txt'])
    })
    await answer('read_status', initial)
    expect(statusReads().map((p) => p.cmd)).toEqual(['read_status'])
  })

  it('连着 Stage 两次（第一次的局部重读还在跑）：补跑那一轮把两次的路径一起读', async () => {
    const { result } = await loaded()
    act(() => { void result.current.refreshChanges('C:/r', unstagedA, ['a.txt']) })
    act(() => { void result.current.refreshChanges('C:/r', unstagedA, ['b.txt']) })
    act(() => { void result.current.refreshChanges('C:/r', unstagedA, ['c.txt']) })
    await answer('read_status_paths', { staged: [m('a.txt')], unstaged: [], ignoreCase: true })
    expect(statusReads().map((p) => [p.cmd, p.args.paths])).toEqual([['read_status_paths', ['b.txt', 'c.txt']]])
  })

  it('换了仓库：局部重读不拿别的仓库的状态当「上一次」', async () => {
    const { result } = await loaded()
    act(() => { void result.current.refreshChanges('C:/other', unstagedA, ['a.txt']) })
    expect(statusReads().map((p) => [p.cmd, p.args.path])).toEqual([['read_status', 'C:/other']])
  })
})

// 自己写的 index 不再触发 `repo-changed`（方案 B），原来靠那一轮顺带做的事要补上（独立 review 第 2、6 条）
describe('afterIndexWrite：Stage / Unstage 之后补上原来靠监听那一轮做的事', () => {
  it('重查 merge driver：暂存的 .gitattributes 会改变检查结果', () => {
    const { result } = renderHook(() => useRepoStore())
    act(() => { result.current.afterIndexWrite('C:/r') })
    expect(pending.filter((p) => p.cmd === 'check_merge_drivers').map((p) => p.args.path)).toEqual(['C:/r'])
  })

  it('同一个仓库开在另一个标签里：那个标签全量刷新，自己这个不重复读', async () => {
    const a = await loaded()
    const b = await loaded()
    const other = renderHook(() => useRepoStore())
    act(() => { void other.result.current.refreshStatus('C:/elsewhere') })
    await answer('read_status', initial)
    pending.length = 0

    act(() => { a.result.current.afterIndexWrite('c:\\r') })
    // 只有 b 读状态（全量）：a 自己刚就地刷新过，别的仓库的标签不管
    expect(statusReads().map((p) => [p.cmd, p.args.path])).toEqual([['read_status', 'C:/r']])
    b.unmount()
    pending.length = 0
    act(() => { a.result.current.afterIndexWrite('C:/r') })
    expect(statusReads()).toEqual([])
  })
})
