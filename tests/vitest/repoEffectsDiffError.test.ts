// @vitest-environment jsdom
//
// 第 72 条：按选中刷新 diff 的 effect 没有 catch。read_diff 失败时是一个未处理的 rejection，
// 顶部错误条什么都不显示，界面上只剩一个空列表。
import { act, renderHook, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string) => cmd === 'read_diff'
    ? Promise.reject({ code: 'git_cli', message: 'fatal: bad object deadbeef' })
    : new Promise(() => {}),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => new Promise(() => {}) }))

const { useRepoEffects, useRepoStore } = await import('@/store/repoStore')

describe('useRepoEffects：按选中读 diff 失败', () => {
  it('错误交给顶部错误条，不吞掉', async () => {
    const { result } = renderHook(() => {
      const store = useRepoStore()
      const state = { ...store.state, path: 'C:/r' }
      useRepoEffects(state, store, 'changes', 'unstaged', true)
      return store
    })
    await act(async () => {})
    await waitFor(() => expect(result.current.state.error).toContain('bad object deadbeef'))
  })
})
