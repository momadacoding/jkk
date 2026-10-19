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
}

const pending: Pending[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: Record<string, unknown>) =>
    new Promise((resolve) => { pending.push({ cmd, args, resolve }) }),
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
