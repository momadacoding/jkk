// @vitest-environment jsdom
//
// 空闲时的服务器侧核对（2026-11-11 提交慢报告第 7 节）：后台入口 `auditSvnIfDue` 在现场没变时不再
// 发 `svn_server_audit`。判定是纯函数，在 svnSync.test.ts；这里测 store 把「核对开始时的现场」记对了、
// 文件事件接上了、显式核对照旧每次都跑。
import { act, renderHook } from '@testing-library/react'
import { useEffect } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import { SVN_AUDIT_MAX_AGE_MS, SVN_AUDIT_POLL_MS } from '@/engine/svnSync'

const answers = new Map<string, (args: Record<string, unknown>) => unknown>()
const calls: string[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: Record<string, unknown>) => {
    calls.push(cmd)
    const answer = answers.get(cmd)
    return answer ? Promise.resolve().then(() => answer(args)) : new Promise(() => {})
  },
}))

const repoChanged: ((e: { payload: string }) => void)[] = []
const watchError: ((e: { payload: string }) => void)[] = []
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: string }) => void) => {
    if (event === 'repo-changed') repoChanged.push(handler)
    if (event === 'watch-error') watchError.push(handler)
    return Promise.resolve(() => {})
  },
}))

const { useRepoEffects, useRepoStore } = await import('@/store/repoStore')

const SVN = { ...ALL_CAPABILITIES, vcs: 'svn' as const }
const T0 = 1_770_000_000_000
let now = T0

function mount() {
  return renderHook(() => {
    const store = useRepoStore()
    // 不给 load_graph 写回答：打开走到读图就挂住，不发打开时那次显式核对
    useEffect(() => { void store.openRepo('C:/r') }, [])
    const state = { ...store.state, path: 'C:/r', capabilities: SVN }
    useRepoEffects(state, store, 'graph', 'unstaged', true)
    return store
  }).result
}

const audits = () => calls.filter((c) => c === 'svn_server_audit').length

async function settle() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0))
  })
}

type Store = { current: { auditSvnIfDue: (p: string) => Promise<void>, refreshSvnAudit: (p: string) => Promise<void> } }

/** 过 `ms` 之后后台触发一次（定时器或窗口焦点）。 */
async function idleTick(store: Store, ms = SVN_AUDIT_POLL_MS) {
  now += ms
  await act(async () => { await store.current.auditSvnIfDue('C:/r') })
  await settle()
}

let stamp = '1'

beforeEach(() => {
  now = T0
  stamp = '1'
  vi.spyOn(Date, 'now').mockImplementation(() => now)
  calls.length = 0
  repoChanged.length = 0
  watchError.length = 0
  answers.clear()
  answers.set('open_repo', () => ({ path: 'C:/r', head: 'work', headHash: 'h', headParents: [] }))
  answers.set('read_capabilities', () => SVN)
  answers.set('svn_wc_db_stamps', () => [{ wcRoot: 'C:/r', mtimeNs: stamp, size: 1 }])
  answers.set('svn_server_audit', () => ({ status: 'matched' }))
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('后台核对：现场没变就不发', () => {
  it('第一次照发；之后空闲着，定时器每 5 分钟一次都不再发', async () => {
    const store = mount()
    await settle()
    await idleTick(store, 0)
    expect(audits()).toBe(1)
    for (let i = 0; i < 6; i++) await idleTick(store)
    expect(audits()).toBe(1)
  })

  it('wc.db 变了（TortoiseSVN 更新、提交）：下一次发', async () => {
    const store = mount()
    await settle()
    await idleTick(store, 0)
    stamp = '2'
    await idleTick(store)
    expect(audits()).toBe(2)
  })

  it('来过 repo-changed（删了白名单里的文件）：下一次发', async () => {
    const store = mount()
    await settle()
    await idleTick(store, 0)
    await act(async () => {
      for (const h of repoChanged) h({ payload: 'C:/r' })
    })
    await settle()
    await idleTick(store)
    expect(audits()).toBe(2)
  })

  it('别的仓库的 repo-changed 不算', async () => {
    const store = mount()
    await settle()
    await idleTick(store, 0)
    await act(async () => {
      for (const h of repoChanged) h({ payload: 'C:/other' })
    })
    await settle()
    await idleTick(store)
    expect(audits()).toBe(1)
  })

  it('上次没得出结论（断网）：下一次照发', async () => {
    answers.set('svn_server_audit', () => ({ status: 'notChecked', reason: '连不上' }))
    const store = mount()
    await settle()
    await idleTick(store, 0)
    await idleTick(store)
    expect(audits()).toBe(2)
  })

  it('上次核对本身抛错：下一次照发', async () => {
    answers.set('svn_server_audit', () => Promise.reject({ code: 'io', message: 'x' }))
    const store = mount()
    await settle()
    await idleTick(store, 0)
    await idleTick(store)
    expect(audits()).toBe(2)
  })

  it('读不出戳：照发', async () => {
    answers.set('svn_wc_db_stamps', () => Promise.reject({ code: 'io', message: 'x' }))
    const store = mount()
    await settle()
    await idleTick(store, 0)
    await idleTick(store)
    expect(audits()).toBe(2)
  })

  it('离上次超过上限：照发', async () => {
    const store = mount()
    await settle()
    await idleTick(store, 0)
    await idleTick(store, SVN_AUDIT_MAX_AGE_MS)
    expect(audits()).toBe(2)
  })

  it('戳是在核对开始之前读的：核对期间 wc.db 变了，下一次照发', async () => {
    let release!: () => void
    answers.set('svn_server_audit', () => new Promise((r) => { release = () => r({ status: 'matched' }) }))
    const store = mount()
    await settle()
    const first = act(async () => { await store.current.auditSvnIfDue('C:/r') })
    await settle()
    stamp = '2'
    release()
    await first
    answers.set('svn_server_audit', () => ({ status: 'matched' }))
    await idleTick(store)
    expect(audits()).toBe(2)
  })
})

describe('监听出错（独立 review 第 5 条）', () => {
  it('watch-error 之后文件事件可能漏了：下一次照发', async () => {
    const store = mount()
    await settle()
    await idleTick(store, 0)
    await act(async () => {
      for (const h of watchError) h({ payload: 'notify: buffer overflow' })
    })
    await settle()
    await idleTick(store)
    expect(audits()).toBe(2)
  })
})

describe('读戳的频率（独立 review 第 3 条）', () => {
  it('空闲时被跳过的那几次也算：一分钟里来回切窗口只读一次戳', async () => {
    const store = mount()
    await settle()
    await idleTick(store, 0)
    await idleTick(store)
    const reads = calls.filter((c) => c === 'svn_wc_db_stamps').length
    for (let i = 0; i < 5; i++) await idleTick(store, 1000)
    expect(calls.filter((c) => c === 'svn_wc_db_stamps').length).toBe(reads)
  })
})

describe('显式核对不受这道闸限制', () => {
  it('显式核对之前没有记下戳（失同步检测失败了）：下一次后台照跑，宁可多跑', async () => {
    const store = mount()
    await settle()
    await act(async () => { await store.current.refreshSvnAudit('C:/r') })
    await idleTick(store)
    expect(audits()).toBe(2)
  })

  it('显式核对一开始就算「在跑」：它还没回来时后台触发不再并发一轮（独立 review 第 1 条）', async () => {
    // 读戳挂住：原来显式核对要先等这一次读戳才记下「开始了」，后台触发正好落在这个空档里
    const reads: (() => void)[] = []
    answers.set('svn_wc_db_stamps', () => new Promise((r) => {
      reads.push(() => r([{ wcRoot: 'C:/r', mtimeNs: stamp, size: 1 }]))
    }))
    answers.set('svn_server_audit', () => new Promise(() => {}))
    const store = mount()
    await settle()
    act(() => { void store.current.refreshSvnAudit('C:/r') })
    act(() => { void store.current.auditSvnIfDue('C:/r') })
    await settle()
    await act(async () => { for (const r of reads) r() })
    await settle()
    expect(audits()).toBe(1)
  })

  it('后台那一轮比显式的晚回来：旧结论不盖新结论（独立 review 第 2 条）', async () => {
    const pending: ((v: unknown) => void)[] = []
    answers.set('svn_server_audit', () => new Promise((r) => { pending.push(r) }))
    const store = mount() as unknown as Store & { current: { state: { svnAudit: unknown } } }
    await settle()
    // 后台 A 先开跑
    const bg = act(async () => { await store.current.auditSvnIfDue('C:/r') })
    await settle()
    // 重建基线之后的显式 B 随后开跑，先回来
    let explicit!: Promise<void>
    act(() => { explicit = store.current.refreshSvnAudit('C:/r') })
    await settle()
    expect(pending).toHaveLength(2)
    await act(async () => { pending[1]({ status: 'matched' }) })
    await act(async () => { await explicit })
    // A 晚到，带的是重建前的结论
    await act(async () => { pending[0]({ status: 'missing', files: ['Code/A.cs'], total: 1 }) })
    await bg
    await settle()
    expect(store.current.state.svnAudit).toEqual({ status: 'matched' })
  })

  it('换了仓库：上一个仓库的核对现场不算这个仓库的（独立 review 第 4 条）', async () => {
    const store = mount() as unknown as Store & { current: { openRepo: (p: string) => Promise<void> } }
    await settle()
    await idleTick(store, 0)
    expect(audits()).toBe(1)
    // 同一个标签打开另一个仓库（打开挂在读图上，不发显式核对）
    act(() => { void store.current.openRepo('C:/other') })
    await settle()
    await idleTick(store)
    expect(audits()).toBe(2)
  })

  it('打开仓库、重建基线之后的核对照旧每次都跑，并且算作「上次核对」', async () => {
    const store = mount() as unknown as Store & { current: { refreshSvnSync: (p: string) => Promise<void> } }
    await settle()
    // 真实顺序：打开、重建之后先跑一次失同步检测（记下 wc.db 戳），再发显式核对
    answers.set('svn_sync_status', () => ({
      baselineRev: 1, nodeRevs: [1], verdict: { kind: 'inSync' },
      base: { floor: 1, newer: 0, gone: 0 }, headBehindBase: false, behindPaths: [],
    }))
    await act(async () => { await store.current.refreshSvnSync('C:/r') })
    await act(async () => { await store.current.refreshSvnAudit('C:/r') })
    await act(async () => { await store.current.refreshSvnAudit('C:/r') })
    expect(audits()).toBe(2)
    await idleTick(store)
    expect(audits()).toBe(2)
  })
})
