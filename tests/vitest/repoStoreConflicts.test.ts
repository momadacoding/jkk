// @vitest-environment jsdom
//
// 冲突信息跟着 status 刷新：只在 status 里有冲突时才问后端，晚到的旧结果不能盖掉新状态。
import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ConflictInfo, StatusData } from '@/engine/types'

interface Pending {
  cmd: string
  args: Record<string, unknown>
  resolve: (value: unknown) => void
  reject: (err: unknown) => void
}

const pending: Pending[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: Record<string, unknown>) =>
    new Promise((resolve, reject) => { pending.push({ cmd, args, resolve, reject }) }),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useRepoStore } = await import('@/store/repoStore')

function statusWith(conflicted: string[]): StatusData {
  return {
    staged: [],
    unstaged: conflicted.map((path) => ({ path, status: 'conflicted', additions: 0, deletions: 0 })),
  }
}

function info(path: string): ConflictInfo {
  return {
    path,
    code: 'UU',
    base: null,
    ours: null,
    theirs: null,
    case: 'markers',
    reason: null,
    driver: null,
    sameAsHead: false,
  }
}

/** 让第一个还没回的 `cmd` 请求返回。 */
async function answer(cmd: string, value: unknown) {
  const call = pending.find((p) => p.cmd === cmd)
  if (!call) throw new Error(`没有 ${cmd} 请求`)
  pending.splice(pending.indexOf(call), 1)
  await act(async () => { call.resolve(value) })
}

beforeEach(() => { pending.length = 0 })

describe('冲突信息的刷新', () => {
  it('status 里有冲突时读 read_conflicts 并写回', async () => {
    const { result } = renderHook(() => useRepoStore())
    let done!: Promise<void>
    act(() => { done = result.current.refreshStatus('C:/r') })
    await answer('read_status', statusWith(['a.txt']))
    expect(pending.map((p) => p.cmd)).toEqual(['read_conflicts'])
    expect(pending[0].args).toEqual({ path: 'C:/r' })
    await answer('read_conflicts', [info('a.txt')])
    await act(async () => { await done })
    expect(result.current.state.conflicts?.map((c) => c.path)).toEqual(['a.txt'])
  })

  it('没有冲突时不调后端，直接置空', async () => {
    const { result } = renderHook(() => useRepoStore())
    let done!: Promise<void>
    act(() => { done = result.current.refreshStatus('C:/r') })
    await answer('read_status', statusWith([]))
    await act(async () => { await done })
    expect(pending).toEqual([])
    expect(result.current.state.conflicts).toEqual([])
  })

  it('冲突解决之后，上一次还没回的冲突信息晚到时丢掉', async () => {
    const { result } = renderHook(() => useRepoStore())
    act(() => { void result.current.refreshStatus('C:/r') })
    await answer('read_status', statusWith(['a.txt']))
    // 第一份 read_conflicts 还在路上，用户在外面把冲突解决了
    act(() => { void result.current.refreshStatus('C:/r') })
    await answer('read_status', statusWith([]))
    expect(result.current.state.conflicts).toEqual([])
    await answer('read_conflicts', [info('a.txt')])
    expect(result.current.state.conflicts).toEqual([])
  })

  it('读取失败时退回「还没读到」，并把错误抛给调用方', async () => {
    const { result } = renderHook(() => useRepoStore())
    let done!: Promise<void>
    act(() => { done = result.current.refreshStatus('C:/r') })
    await answer('read_status', statusWith(['a.txt']))
    const call = pending.find((p) => p.cmd === 'read_conflicts')!
    pending.splice(pending.indexOf(call), 1)
    await act(async () => {
      call.reject({ code: 'git_cli', message: 'boom' })
      await expect(done).rejects.toEqual({ code: 'git_cli', message: 'boom' })
    })
    expect(result.current.state.conflicts).toBeNull()
  })
})
