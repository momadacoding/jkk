// @vitest-environment jsdom
//
// 第 71 条：SVN 的几条刷新返回后无条件写 state。它们可能耗时数秒（fetchSvnLog 走网络），
// 期间切到别的仓库，旧仓库的结果会写进新仓库：错误的失同步横幅、错误的 SVN 节点接进提交图。
import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { SyncStatus } from '@/engine/svnSync'

interface Pending {
  cmd: string
  args: Record<string, unknown>
  resolve: (value: unknown) => void
}

const pending: Pending[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: Record<string, unknown>) =>
    new Promise((resolve) => { pending.push({ cmd, args, resolve }) }),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useRepoStore } = await import('@/store/repoStore')

beforeEach(() => { pending.length = 0 })

/** 让某个仓库的某条命令返回。 */
async function answer(cmd: string, path: string, value: unknown) {
  const call = pending.find((p) => p.cmd === cmd && p.args.path === path)
  if (!call) throw new Error(`没有 ${path} 的 ${cmd} 请求：${pending.map((p) => p.cmd).join(', ')}`)
  pending.splice(pending.indexOf(call), 1)
  await act(async () => { call.resolve(value) })
}

/** 打开仓库到 state.path 写好为止；之后的刷新一律挂着不答。 */
async function openAt(result: { current: ReturnType<typeof useRepoStore> }, path: string) {
  act(() => { void result.current.openRepo(path) })
  await answer('open_repo', path, { path, head: 'main', headHash: 'h' })
  await answer('read_capabilities', path, { vcs: 'git' })
  expect(result.current.state.path).toBe(path)
}

const logPage = { revs: [{ rev: 7, author: 'a', date: '2026-01-01T00:00:00Z', message: 'r7' }], complete: true }

const syncStatus = {
  baselineRev: 7,
  nodeRevs: [7],
  verdict: { kind: 'InSync' },
  base: { floor: 7, ceiling: 7, mixed: false },
  headBehindBase: false,
} as unknown as SyncStatus

describe('SVN 刷新：请求在飞时切到了别的仓库', () => {
  it('fetchSvnLog：旧仓库晚到的 SVN 历史不接进新仓库的图', async () => {
    const { result } = renderHook(() => useRepoStore())
    await openAt(result, 'C:/a')
    act(() => { void result.current.fetchSvnLog('C:/a') })
    await openAt(result, 'C:/b')
    await answer('svn_fetch_log', 'C:/a', logPage)
    expect(result.current.state.svnRevs).toEqual([])
    expect(result.current.state.svnLogComplete).toBe(false)
  })

  it('refreshSvnLog：同上', async () => {
    const { result } = renderHook(() => useRepoStore())
    await openAt(result, 'C:/a')
    act(() => { void result.current.refreshSvnLog('C:/a') })
    await openAt(result, 'C:/b')
    await answer('svn_read_log', 'C:/a', logPage)
    expect(result.current.state.svnRevs).toEqual([])
  })

  it('refreshSvnSync：旧仓库的同步结论不写进新仓库', async () => {
    const { result } = renderHook(() => useRepoStore())
    await openAt(result, 'C:/a')
    act(() => { void result.current.refreshSvnSync('C:/a') })
    await answer('svn_root_revs', 'C:/a', [{ wcRoot: 'C:/a', rev: 7 }])
    await openAt(result, 'C:/b')
    await answer('svn_sync_status', 'C:/a', syncStatus)
    expect(result.current.state.svnSync).toBeNull()
  })

  it('同一个仓库照常写回', async () => {
    const { result } = renderHook(() => useRepoStore())
    await openAt(result, 'C:/a')
    act(() => { void result.current.fetchSvnLog('C:/a') })
    await answer('svn_fetch_log', 'C:/a', logPage)
    expect(result.current.state.svnRevs.map((r) => r.rev)).toEqual([7])

    act(() => { void result.current.refreshSvnSync('C:/a') })
    await answer('svn_root_revs', 'C:/a', [{ wcRoot: 'C:/a', rev: 7 }])
    await answer('svn_sync_status', 'C:/a', syncStatus)
    expect(result.current.state.svnSync).toEqual(syncStatus)
  })

  it('路径写法不同（大小写、反斜杠）仍算同一个仓库', async () => {
    const { result } = renderHook(() => useRepoStore())
    await openAt(result, 'C:/Work/Repo')
    act(() => { void result.current.fetchSvnLog('c:\\work\\repo') })
    await answer('svn_fetch_log', 'c:\\work\\repo', logPage)
    expect(result.current.state.svnRevs.map((r) => r.rev)).toEqual([7])
  })
})
