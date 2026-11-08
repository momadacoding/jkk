// @vitest-environment jsdom
//
// 第 70 条：基线分支丢了时失同步判定报 svn_no_baseline。原来这个失败被吞掉，界面上什么都不挂，
// 用户在应用里没有出路。store 要把它记下来，交给恢复横幅。
import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import type { SyncStatus } from '@/engine/svnSync'

/** 命令 → 回答。没写的命令挂住不回。 */
const answers = new Map<string, () => unknown>()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string) => {
    const answer = answers.get(cmd)
    return answer ? Promise.resolve().then(answer) : new Promise(() => {})
  },
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useRepoStore } = await import('@/store/repoStore')

const SVN = { ...ALL_CAPABILITIES, vcs: 'svn' as const }
const inSync: SyncStatus = {
  baselineRev: 1,
  nodeRevs: [1],
  verdict: { kind: 'inSync' },
  base: { floor: 1, newer: 0, gone: 0 },
  headBehindBase: false,
}

beforeEach(() => {
  answers.clear()
  answers.set('open_repo', () => ({ path: 'C:/r', head: 'work', headHash: 'h' }))
  answers.set('read_capabilities', () => SVN)
  answers.set('svn_wc_db_stamps', () => [{ wcRoot: 'C:/r', mtimeNs: '1', size: 1 }])
})

async function opened() {
  const hook = renderHook(() => useRepoStore())
  await act(async () => { void hook.result.current.openRepo('C:/r') })
  expect(hook.result.current.state.path).toBe('C:/r')
  return hook
}

describe('失同步判定报 svn_no_baseline', () => {
  it('记成基线缺失，旧结论作废', async () => {
    const { result } = await opened()
    answers.set('svn_sync_status', () => inSync)
    await act(async () => { await result.current.refreshSvnSync('C:/r') })
    expect(result.current.state.svnSync).toEqual(inSync)

    // wc.db 也变了（refreshSvnSync 本身不看戳，无条件跑第二档）
    answers.set('svn_wc_db_stamps', () => [{ wcRoot: 'C:/r', mtimeNs: '2', size: 1 }])
    answers.set('svn_sync_status', () => Promise.reject({ code: 'svn_no_baseline', message: '读不到基线分支' }))
    await act(async () => { await result.current.refreshSvnSync('C:/r').catch(() => undefined) })
    expect(result.current.state.svnBaselineMissing).toBe(true)
    expect(result.current.state.svnSync).toBeNull()
  })

  it('基线确实丢了之后来一次别的失败（锁、网络）：标记不清，恢复横幅不闪掉', async () => {
    const { result } = await opened()
    answers.set('svn_sync_status', () => Promise.reject({ code: 'svn_no_baseline', message: '读不到基线分支' }))
    await act(async () => { await result.current.refreshSvnSync('C:/r').catch(() => undefined) })
    answers.set('svn_wc_db_stamps', () => [{ wcRoot: 'C:/r', mtimeNs: '2', size: 1 }])
    answers.set('svn_sync_status', () => Promise.reject({ code: 'svn_wc_locked', message: 'E155004' }))
    await act(async () => { await result.current.refreshSvnSync('C:/r').catch(() => undefined) })
    expect(result.current.state.svnBaselineMissing).toBe(true)
    expect(result.current.state.svnSync).toBeNull()
  })

  it('别的失败不算基线缺失，上一次的结论保留', async () => {
    const { result } = await opened()
    answers.set('svn_sync_status', () => inSync)
    await act(async () => { await result.current.refreshSvnSync('C:/r') })
    answers.set('svn_wc_db_stamps', () => [{ wcRoot: 'C:/r', mtimeNs: '2', size: 1 }])
    answers.set('svn_sync_status', () => Promise.reject({ code: 'svn_cli', message: 'E170013' }))
    await act(async () => { await result.current.refreshSvnSync('C:/r').catch(() => undefined) })
    expect(result.current.state.svnBaselineMissing).toBe(false)
    expect(result.current.state.svnSync).toEqual(inSync)
  })
})
