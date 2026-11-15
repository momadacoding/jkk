// @vitest-environment jsdom
//
// 同步（跟进、更新、交付）之后的强制复查（2026-11-11 使用报告「跟进耗时」10.5 的 P3）。
//
// 后端的同步命令返回前会补发一次 `repo-changed`（`with_watch_suspended`）。同步多半挪了 HEAD，监听读到
// 新 HEAD 就强制跑一轮第二档；流程收尾的 `refreshSvnSync` 又强制跑一轮。两轮各是一遍 `svn status`
// 全量扫描，结论相同。这里测 store 让两者共用一轮，而且只在安全时共用。
import { act, renderHook } from '@testing-library/react'
import { useEffect } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ALL_CAPABILITIES } from '@/engine/capabilities'

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
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: string }) => void) => {
    if (event === 'repo-changed') repoChanged.push(handler)
    return Promise.resolve(() => {})
  },
}))

const { useRepoEffects, useRepoStore } = await import('@/store/repoStore')

const SVN = { ...ALL_CAPABILITIES, vcs: 'svn' as const }
const O = 'o'.repeat(40)
const P = 'p'.repeat(40)
const N = 'n'.repeat(40)
const B = 'b'.repeat(40)

const IN_SYNC = {
  baselineRev: 1,
  nodeRevs: [1],
  verdict: { kind: 'inSync' },
  base: { floor: 1, newer: 0, gone: 0 },
  headBehindBase: false,
  behindPaths: [],
}

type Head = { path: string, head: string, headHash: string, headParents: string[] }
const headAt = (branch: string, hash: string, parents: string[]): Head =>
  ({ path: 'C:/r', head: branch, headHash: hash, headParents: parents })

/** 磁盘上的 HEAD：之后的 `open_repo` 都读到它。 */
function head(branch: string, hash: string, parents: string[]) {
  answers.set('open_repo', () => headAt(branch, hash, parents))
}

function mount() {
  return renderHook(() => {
    const store = useRepoStore()
    // 不给 load_graph 写回答时，打开走到读图就挂住，只为让 store 记下打开的路径
    useEffect(() => { void store.openRepo('C:/r') }, [])
    const state = { ...store.state, path: 'C:/r', capabilities: SVN }
    useRepoEffects(state, store, 'graph', 'unstaged', true)
    return store
  })
}

const syncChecks = () => calls.filter((c) => c === 'svn_sync_status').length

async function settle() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0))
  })
}

/** 一道闸：`hold` 之后的回答都等 `release`。 */
function gate() {
  let release!: () => void
  const opened = new Promise<void>((r) => { release = r })
  return { opened, release: () => act(async () => { release() }) }
}

/** 打开仓库并让 HEAD P 校验成功一次：之后的 HEAD 变化都和它比，wc.db 戳也记下了。 */
async function checkedAtP() {
  head('work', P, [O])
  const view = mount()
  await settle()
  await act(async () => {
    for (const h of repoChanged) h({ payload: 'C:/r' })
  })
  await settle()
  const before = syncChecks()
  expect(before).toBeGreaterThan(0)
  return { store: view.result, before }
}

type Store = { current: { refreshSvnSync: (path: string) => Promise<void> } }

/**
 * 同步命令返回的那一刻：后端先补发 `repo-changed`，命令的结果紧跟着回来，流程在同一拍里调
 * `refreshSvnSync`。复查的 Promise 包在对象里交回：直接 return 的话 async 函数会等它落定。
 */
async function syncReturned(store: Store) {
  let check!: Promise<void>
  await act(async () => {
    for (const h of repoChanged) h({ payload: 'C:/r' })
    check = store.current.refreshSvnSync('C:/r')
    check.catch(() => undefined)
  })
  return { check }
}

beforeEach(() => {
  calls.length = 0
  repoChanged.length = 0
  answers.clear()
  answers.set('read_capabilities', () => SVN)
  // 戳不变：第一档不触发第二档，下面数到的都是强制的那几轮
  answers.set('svn_wc_db_stamps', () => [{ wcRoot: 'C:/r', mtimeNs: '1', size: 1 }])
  answers.set('svn_sync_status', () => IN_SYNC)
})

describe('同步之后的强制复查与补发的 repo-changed 共用一轮', () => {
  it('复查在跑时来了 repo-changed（补发的那次晚到、或同步锁释放引起的那次）：看到同一个 HEAD 已经在校验，不再排一轮', async () => {
    const { store, before } = await checkedAtP()
    // 跟进把 work 重放到了新基线上：新 HEAD 不是旧 HEAD 的子提交
    head('work', N, [B])
    const running = gate()
    answers.set('svn_sync_status', () => running.opened.then(() => IN_SYNC))

    let check!: Promise<void>
    await act(async () => { check = store.current.refreshSvnSync('C:/r') })
    await settle()
    expect(syncChecks()).toBe(before + 1)
    await act(async () => {
      for (const h of repoChanged) h({ payload: 'C:/r' })
    })
    await settle()
    await running.release()
    await check
    await settle()

    expect(syncChecks()).toBe(before + 1)
  })

  it('监听那一轮先开跑（比戳的那一轮先收尾，监听才读到 HEAD）：复查复用它，不再补跑', async () => {
    const { store, before } = await checkedAtP()
    // 读 HEAD 先挂住：监听那边「戳没变」的一轮先收尾，之后监听与复查才各自读到新 HEAD
    const reading = gate()
    answers.set('open_repo', () => reading.opened.then(() => headAt('work', N, [B])))

    const { check } = await syncReturned(store)
    await settle()
    expect(syncChecks()).toBe(before)
    await reading.release()
    await check
    await settle()

    expect(syncChecks()).toBe(before + 1)
  })

  it('复查写回后记下了 HEAD：之后的事件（存盘、尾随复查）不再跑', async () => {
    const { store, before } = await checkedAtP()
    head('work', N, [B])
    await (await syncReturned(store)).check
    await settle()
    expect(syncChecks()).toBe(before + 1)

    await act(async () => {
      for (const h of repoChanged) h({ payload: 'C:/r' })
    })
    await settle()
    expect(syncChecks()).toBe(before + 1)
  })

  it('HEAD 没动的同步（重建基线）：照样强制跑一轮', async () => {
    const { store, before } = await checkedAtP()
    await (await syncReturned(store)).check
    await settle()
    expect(syncChecks()).toBe(before + 1)
  })
})

describe('不该共用的时候', () => {
  it('复查进来之前就在跑的强制一轮不复用：它可能开跑在同步落地之前，要再跑一轮', async () => {
    const { store, before } = await checkedAtP()
    // 同步还没返回时外部先挪了 HEAD：监听为它强制的那一轮挂住不回
    head('work', N, [B])
    const running = gate()
    answers.set('svn_sync_status', () => running.opened.then(() => IN_SYNC))
    await act(async () => {
      for (const h of repoChanged) h({ payload: 'C:/r' })
    })
    await settle()
    expect(syncChecks()).toBe(before + 1)

    // 这时同步返回，流程调复查：HEAD 相同，但那一轮是早先开跑的
    let check!: Promise<void>
    await act(async () => { check = store.current.refreshSvnSync('C:/r') })
    await settle()
    await running.release()
    await check
    await settle()

    expect(syncChecks()).toBe(before + 2)
  })

  it('读不出 HEAD：照旧强制跑一轮，不因为没有 HEAD 就没有结论', async () => {
    const { store, before } = await checkedAtP()
    answers.set('open_repo', () => Promise.reject({ code: 'io', message: '读不了' }))
    let check!: Promise<void>
    await act(async () => { check = store.current.refreshSvnSync('C:/r') })
    await check
    await settle()
    expect(syncChecks()).toBe(before + 1)
  })

  it('在跑的那一轮是替另一个 HEAD 开跑的（读 HEAD 的这一拍里 HEAD 又动了）：不复用，按自己读到的再跑一轮（独立 review 第 3 条）', async () => {
    const { store, before } = await checkedAtP()
    // 监听先读到 N、为它开跑；流程随后读到的已经是 M。复用的话 M 会被记成「按它校验过」，
    // 而那一轮的结论是按 N 算的
    const first = gate()
    const second = gate()
    let reads = 0
    answers.set('open_repo', () => {
      reads += 1
      return reads === 1
        ? first.opened.then(() => headAt('work', N, [B]))
        : second.opened.then(() => headAt('work', 'm'.repeat(40), [N]))
    })
    const running = gate()
    answers.set('svn_sync_status', () => running.opened.then(() => IN_SYNC))

    const { check } = await syncReturned(store)
    await settle()
    await first.release()
    await settle()
    expect(syncChecks()).toBe(before + 1)
    await second.release()
    await settle()
    await running.release()
    await check
    await settle()

    expect(syncChecks()).toBe(before + 2)
  })

  it('复查失败一次：自己再试一次。与监听共用一轮之后，那一轮失败就没有第二个人会再试（独立 review 第 6 条）', async () => {
    const { store, before } = await checkedAtP()
    head('work', N, [B])
    let failed = false
    answers.set('svn_sync_status', () => {
      if (failed) return IN_SYNC
      failed = true
      return Promise.reject({ code: 'svn_failed', message: 'wc.db 被锁' })
    })

    await (await syncReturned(store)).check
    await settle()

    expect(syncChecks()).toBe(before + 2)
    // 第二次写回并记下了 HEAD：再来事件不再跑
    await act(async () => {
      for (const h of repoChanged) h({ payload: 'C:/r' })
    })
    await settle()
    expect(syncChecks()).toBe(before + 2)
  })

  it('两次都失败：错误交给调用方（流程写进错误条）', async () => {
    const { store, before } = await checkedAtP()
    head('work', N, [B])
    answers.set('svn_sync_status', () => Promise.reject({ code: 'svn_failed', message: 'wc.db 被锁' }))
    let check!: Promise<void>
    await act(async () => {
      check = store.current.refreshSvnSync('C:/r')
      check.catch(() => undefined)
    })
    await expect(check).rejects.toMatchObject({ code: 'svn_failed' })
    expect(syncChecks()).toBe(before + 2)
  })
})
