// @vitest-environment jsdom
//
// 提交后的 SVN 第二档（2026-11-11 提交慢排查 H1）：本应用的普通提交成功后 `noteOwnCommit` 现读 HEAD，
// 是叠在已校验 HEAD 上的单父提交就直接记下，repo-changed 随后读到它时不再跑 `svn_sync_status`；
// 其余 HEAD 变化照旧跑。判定是纯函数，在 svnHeadMove.test.ts；这里测 store 把读取、记录、
// 与还没落定的那一轮的衔接接对了。
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
const M = 'm'.repeat(40)

const IN_SYNC = {
  baselineRev: 1,
  nodeRevs: [1],
  verdict: { kind: 'inSync' },
  base: { floor: 1, newer: 0, gone: 0 },
  headBehindBase: false,
  behindPaths: [],
}

/** 磁盘上的 HEAD：之后的 `open_repo` 都读到它。 */
function head(branch: string, hash: string, parents: string[]) {
  answers.set('open_repo', () => ({ path: 'C:/r', head: branch, headHash: hash, headParents: parents }))
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

async function fireRepoChanged() {
  await act(async () => {
    for (const h of repoChanged) h({ payload: 'C:/r' })
  })
  await settle()
}

/** 打开仓库并让 HEAD P 校验成功一次：之后的 HEAD 变化都和它比。 */
async function checkedAtP() {
  // 先给 open_repo 写回答：打开要走到记下 openedPath，否则校验结论全被当成「已切走」丢掉
  head('work', P, [O])
  const view = mount()
  await settle()
  await fireRepoChanged()
  const before = syncChecks()
  expect(before).toBeGreaterThan(0)
  return { store: view.result, before }
}

/** 本应用提交成功：磁盘上的 HEAD 已经是新提交，`handleCommit` 调 `noteOwnCommit`。 */
async function ownCommit(store: { current: { noteOwnCommit: (path: string) => Promise<void> } }) {
  await act(async () => { await store.current.noteOwnCommit('C:/r') })
}

beforeEach(() => {
  calls.length = 0
  repoChanged.length = 0
  answers.clear()
  answers.set('read_capabilities', () => SVN)
  // 戳不变：第一档不触发第二档，下面数到的都是 HEAD 变化引起的
  answers.set('svn_wc_db_stamps', () => [{ wcRoot: 'C:/r', mtimeNs: '1', size: 1 }])
  answers.set('svn_sync_status', () => IN_SYNC)
})

describe('提交之后 HEAD 变了', () => {
  it('本应用在已校验 HEAD 上的普通提交：不跑第二档', async () => {
    const { store, before } = await checkedAtP()
    head('work', N, [P])
    await ownCommit(store)
    await fireRepoChanged()
    expect(syncChecks()).toBe(before)
  })

  it('记下的是新 HEAD：再在它上面提交也不跑', async () => {
    const { store, before } = await checkedAtP()
    head('work', N, [P])
    await ownCommit(store)
    await fireRepoChanged()
    head('work', M, [N])
    await ownCommit(store)
    await fireRepoChanged()
    expect(syncChecks()).toBe(before)
  })

  it('不是本应用的提交（外部工具、快进到基线）：照旧跑', async () => {
    const { before } = await checkedAtP()
    head('work', N, [P])
    await fireRepoChanged()
    expect(syncChecks()).toBe(before + 1)
  })

  it('提交后读到的是 merge 提交（两个父提交）：照旧跑', async () => {
    const { store, before } = await checkedAtP()
    head('work', N, [P, O])
    await ownCommit(store)
    await fireRepoChanged()
    expect(syncChecks()).toBe(before + 1)
  })

  it('监听早于 noteOwnCommit：已经为新 HEAD 排了一轮，只跑这一轮', async () => {
    const { store, before } = await checkedAtP()
    head('work', N, [P])
    await fireRepoChanged()
    await ownCommit(store)
    await fireRepoChanged()
    expect(syncChecks()).toBe(before + 1)
  })
})

describe('HEAD 没换提交', () => {
  it('在当前提交上 checkout -b 新分支：不跑', async () => {
    const { before } = await checkedAtP()
    head('feature', P, [O])
    await fireRepoChanged()
    expect(syncChecks()).toBe(before)
  })

  it('同一个提交切到基线分支：跑', async () => {
    const { before } = await checkedAtP()
    head('svn/base', P, [O])
    await fireRepoChanged()
    expect(syncChecks()).toBe(before + 1)
  })
})

describe('提交时那一轮校验还没落定（独立 review 第 8 条）', () => {
  it('那一轮写回后直接记成这次提交：之后的事件都不再跑', async () => {
    // 先在 O 上校验成功一次：wc.db 戳也记下了，下面数到的只有 HEAD 引起的校验
    head('work', O, [])
    const { result: store } = mount()
    await settle()
    await fireRepoChanged()
    const settledAtO = syncChecks()
    // 外部切到 P：为 P 强制的那一轮挂住不回
    let release!: () => void
    const gate = new Promise<void>((r) => { release = r })
    answers.set('svn_sync_status', () => gate.then(() => IN_SYNC))
    head('work', P, [O])
    await fireRepoChanged()
    expect(syncChecks()).toBe(settledAtO + 1)
    // 那一轮还在跑时本应用在 P 上提交了 N
    head('work', N, [P])
    await ownCommit(store)
    await fireRepoChanged()
    release()
    await settle()
    // 再来一次事件（存盘、尾随复查）：HEAD 还是 N，已经按 N 记下
    await fireRepoChanged()
    // 自始至终只有 P 那一轮：N 既没在 P 落定前排一轮，也没在之后补一轮
    expect(syncChecks()).toBe(settledAtO + 1)
  })

  it('那一轮失败了：N 的事件已经被吞掉，要替它补一轮（第二轮独立 review 第 1 条）', async () => {
    head('work', O, [])
    const { result: store } = mount()
    await settle()
    await fireRepoChanged()
    const settledAtO = syncChecks()
    let fail!: () => void
    const gate = new Promise<never>((_, rej) => { fail = () => rej({ code: 'svn_failed', message: 'wc.db 被锁' }) })
    answers.set('svn_sync_status', () => gate)
    head('work', P, [O])
    await fireRepoChanged()
    head('work', N, [P])
    await ownCommit(store)
    await fireRepoChanged()
    // P 那一轮失败；之后的校验照常成功
    answers.set('svn_sync_status', () => IN_SYNC)
    await act(async () => { fail() })
    await settle()
    // P 那一轮 + 替 N 补的一轮
    expect(syncChecks()).toBe(settledAtO + 2)
    // 补的那轮记下了 N：再来事件不再跑
    await fireRepoChanged()
    expect(syncChecks()).toBe(settledAtO + 2)
  })
})

describe('打开仓库', () => {
  it('打开时那次校验记下 HEAD：打开后第一件事就是提交，也不再跑第二档', async () => {
    head('work', P, [O])
    // 让 openRepo 一路走到 SVN 那段（打开时的强制校验）
    answers.set('load_graph', () => ({ commits: [], refs: [] }))
    answers.set('read_status', () => ({ staged: [], unstaged: [] }))
    answers.set('read_conflicts', () => [])
    answers.set('read_worktrees_cmd', () => [])
    answers.set('read_upstream', () => null)
    answers.set('read_stashes', () => [])
    const { result: store } = mount()
    await settle()
    const before = syncChecks()
    expect(before).toBe(1)
    head('work', N, [P])
    await ownCommit(store)
    await fireRepoChanged()
    expect(syncChecks()).toBe(before)
  })
})
