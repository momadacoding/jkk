// @vitest-environment jsdom
//
// 焦点检测与失焦快照的接线（分析 §6.6，日志 W26、Review 2 遗留）。判定是纯函数，在 svnSync.test.ts；
// 这里测 store 与 effect 把它们接对了：什么时候发哪条命令，校验在跑时不写快照，只处理当前标签。
// 真实窗口里焦点事件的时序测不到，见日志 Task 11 的手测清单。
import { act, renderHook } from '@testing-library/react'
import { useEffect } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import type { SyncVerdict } from '@/engine/svnSync'

/** 命令 → 回答。没写的命令挂住不回，和真后端还没答一样。 */
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

function stamps(mtimeNs: string) {
  return () => [{ wcRoot: 'C:/r', mtimeNs, size: 1 }]
}

function syncStatus(verdict: SyncVerdict) {
  return () => ({
    baselineRev: 1,
    nodeRevs: [1],
    verdict,
    base: { floor: 1, newer: 0, gone: 0 },
    headBehindBase: false,
  })
}

const IN_SYNC: SyncVerdict = { kind: 'inSync' }
const ADVANCED: SyncVerdict = {
  kind: 'externallyAdvanced',
  changed: [{ path: 'Code/A.cs', rev: 2, author: 'zhang' }],
  added: [],
  removed: [],
}

function mount(active = true) {
  return renderHook(
    ({ active }) => {
      const store = useRepoStore()
      // store 自己也得认为打开的是 C:/r：写回前的 `samePath` 守卫核对的是 store 里的路径
      // （第 71 条），只把 path 塞给 effect 的话写回全被当成「已切到别的仓库」丢掉。
      // 打开走到 refreshGraph 就挂住（没写回答），不会再发任何 svn 命令
      useEffect(() => { void store.openRepo('C:/r') }, [])
      const state = { ...store.state, path: 'C:/r', capabilities: SVN }
      useRepoEffects(state, store, 'graph', 'unstaged', active)
      return store
    },
    { initialProps: { active } },
  )
}

const count = (cmd: string) => calls.filter((c) => c === cmd).length

async function fire(event: 'focus' | 'blur') {
  await act(async () => {
    window.dispatchEvent(new Event(event))
    // 让挂在这次事件上的几次 invoke 都走完
    await new Promise((r) => setTimeout(r, 0))
  })
}

/** 焦点检测跑完一轮：下面的失焦才有「最近一次判定」。 */
async function settleOn(verdict: SyncVerdict) {
  answers.set('svn_sync_status', syncStatus(verdict))
  await fire('focus')
  expect(count('svn_sync_status')).toBeGreaterThan(0)
}

beforeEach(() => {
  calls.length = 0
  repoChanged.length = 0
  answers.clear()
  answers.set('open_repo', () => ({ path: 'C:/r', head: 'work', headHash: 'h' }))
  answers.set('read_capabilities', () => SVN)
  answers.set('svn_root_revs', () => [])
  answers.set('svn_wc_db_stamps', stamps('1'))
  answers.set('svn_follow_mark', () => undefined)
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('窗口拿到焦点', () => {
  it('第一次没有记录：要校验', async () => {
    mount()
    answers.set('svn_sync_status', syncStatus(IN_SYNC))
    await fire('focus')
    expect(count('svn_wc_db_stamps')).toBe(1)
    expect(count('svn_sync_status')).toBe(1)
  })

  it('wc.db 没变：只有 stat，不发校验', async () => {
    mount()
    await settleOn(IN_SYNC)
    await fire('focus')
    await fire('focus')
    expect(count('svn_wc_db_stamps')).toBe(3)
    expect(count('svn_sync_status')).toBe(1)
  })

  it('wc.db 变了：校验，横幅跟着变', async () => {
    const { result } = mount()
    await settleOn(IN_SYNC)

    answers.set('svn_wc_db_stamps', stamps('2'))
    answers.set('svn_sync_status', syncStatus(ADVANCED))
    await fire('focus')

    expect(count('svn_sync_status')).toBe(2)
    expect(result.current.state.svnSync?.verdict.kind).toBe('externallyAdvanced')
  })

  it('校验失败：下一次焦点重试，wc.db 没再变也要重试', async () => {
    mount()
    answers.set('svn_sync_status', () => {
      throw { code: 'svn_failed', message: 'x' }
    })
    await fire('focus')
    answers.set('svn_sync_status', syncStatus(IN_SYNC))
    await fire('focus')
    expect(count('svn_sync_status')).toBe(2)
  })

  it('非当前标签不处理：等切过去再说', async () => {
    mount(false)
    await fire('focus')
    await fire('blur')
    expect(count('svn_wc_db_stamps')).toBe(0)
    expect(count('svn_follow_mark')).toBe(0)
  })
})

describe('窗口失焦', () => {
  it('最近一次判定是 inSync：写快照', async () => {
    mount()
    await settleOn(IN_SYNC)
    await fire('blur')
    expect(count('svn_follow_mark')).toBe(1)
  })

  it('最近一次判定不是 inSync：不写', async () => {
    mount()
    await settleOn(ADVANCED)
    await fire('blur')
    expect(count('svn_follow_mark')).toBe(0)
  })

  it('还没判定过：不写', async () => {
    mount()
    await fire('blur')
    expect(count('svn_follow_mark')).toBe(0)
  })

  it('校验进行中：不写，校验完也不补写', async () => {
    mount()
    await settleOn(IN_SYNC)

    // 外部提交之后切回来，校验还没出结果就又切走了（Review 2 遗留）
    answers.set('svn_wc_db_stamps', stamps('2'))
    let finish!: () => void
    const gate = new Promise<void>((r) => {
      finish = r
    })
    answers.set('svn_sync_status', () => gate.then(syncStatus(IN_SYNC)))
    await fire('focus')
    await fire('blur')
    expect(count('svn_follow_mark')).toBe(0)

    await act(async () => {
      finish()
      await new Promise((r) => setTimeout(r, 0))
    })
    expect(count('svn_follow_mark')).toBe(0)
  })

  it('还在读 wc.db 戳时切走：同样算校验进行中', async () => {
    mount()
    await settleOn(IN_SYNC)

    let release!: (v: unknown) => void
    const held = new Promise((r) => {
      release = r
    })
    answers.set('svn_wc_db_stamps', () => held)
    await fire('focus')
    await fire('blur')

    // 失焦自己也要读戳，放行之后才看得出它有没有写
    await act(async () => {
      release(stamps('1')())
      await new Promise((r) => setTimeout(r, 0))
    })
    expect(count('svn_follow_mark')).toBe(0)
  })

  it('repo-changed 的校验在跑时同样不写', async () => {
    mount()
    await settleOn(IN_SYNC)
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0))
    })

    answers.set('svn_root_revs', () => new Promise(() => {}))
    await act(async () => {
      repoChanged.at(-1)?.({ payload: 'C:/r' })
    })
    await fire('blur')
    expect(count('svn_follow_mark')).toBe(0)
  })

  it('提交在 jkk 前台时完成：wc.db 已变，不写', async () => {
    mount()
    await settleOn(IN_SYNC)
    answers.set('svn_wc_db_stamps', stamps('2'))
    await fire('blur')
    expect(count('svn_follow_mark')).toBe(0)
  })

  it('上一次校验失败：没有判定，不写', async () => {
    // 走 repo-changed：wc.db 的戳没动，挡住快照的只能是「这次判定失败了」
    mount()
    await settleOn(IN_SYNC)
    answers.set('svn_root_revs', () => [{ wcRoot: 'C:/r', rev: 9 }])
    answers.set('svn_sync_status', () => {
      throw { code: 'svn_failed', message: 'x' }
    })
    await act(async () => {
      repoChanged.at(-1)?.({ payload: 'C:/r' })
      await new Promise((r) => setTimeout(r, 0))
    })

    await fire('blur')

    expect(count('svn_follow_mark')).toBe(0)
  })

  it('写不成只记 warn，不打扰用户', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { result } = mount()
    await settleOn(IN_SYNC)
    answers.set('svn_follow_mark', () => {
      throw { code: 'io', message: 'x' }
    })

    await fire('blur')

    expect(warn).toHaveBeenCalled()
    expect(result.current.state.error).toBeNull()
  })
})

describe('校验的先后', () => {
  it('先发起的校验晚到：不能把新结论盖掉', async () => {
    // A 看到的是提交之前的工作副本，B 看到的是提交之后的。A 晚到时盖回 InSync 的话，
    // 横幅消失，下一次失焦还会写一份晚于提交的快照
    const { result } = mount()
    await settleOn(IN_SYNC)

    let finishA!: () => void
    const gate = new Promise<void>((r) => {
      finishA = r
    })
    answers.set('svn_wc_db_stamps', stamps('2'))
    answers.set('svn_sync_status', () => gate.then(syncStatus(IN_SYNC)))
    await fire('focus')

    answers.set('svn_wc_db_stamps', stamps('3'))
    answers.set('svn_sync_status', syncStatus(ADVANCED))
    await fire('focus')
    expect(result.current.state.svnSync?.verdict.kind).toBe('externallyAdvanced')

    await act(async () => {
      finishA()
      await new Promise((r) => setTimeout(r, 0))
    })

    expect(result.current.state.svnSync?.verdict.kind).toBe('externallyAdvanced')
    await fire('blur')
    expect(count('svn_follow_mark')).toBe(0)
  })
})

describe('切标签', () => {
  it('切到这个标签：跑一次焦点检测', async () => {
    const { rerender } = mount(false)
    answers.set('svn_sync_status', syncStatus(IN_SYNC))

    await act(async () => {
      rerender({ active: true })
      await new Promise((r) => setTimeout(r, 0))
    })

    expect(count('svn_wc_db_stamps')).toBe(1)
    expect(count('svn_sync_status')).toBe(1)
  })

  it('切走：等于这个仓库失焦', async () => {
    const { rerender } = mount(true)
    await settleOn(IN_SYNC)

    await act(async () => {
      rerender({ active: false })
      await new Promise((r) => setTimeout(r, 0))
    })

    expect(count('svn_follow_mark')).toBe(1)
  })

  it('标签没切换的重渲染不算', async () => {
    const { rerender } = mount(true)
    await act(async () => {
      rerender({ active: true })
      await new Promise((r) => setTimeout(r, 0))
    })
    expect(count('svn_wc_db_stamps')).toBe(0)
  })
})

describe('HEAD 动了（§6.11.3：切到落后的分支要出「跟进」）', () => {
  async function changed() {
    await act(async () => {
      repoChanged.at(-1)?.({ payload: 'C:/r' })
      await new Promise((r) => setTimeout(r, 0))
    })
  }

  function head(hash: string) {
    answers.set('open_repo', () => ({ head: 'work', headHash: hash }))
  }

  it('切分支不改 revision，第一档看不见：HEAD 变了要跑第二档', async () => {
    mount()
    await settleOn(IN_SYNC)
    head('a'.repeat(40))
    await changed()
    const after = count('svn_sync_status')

    head('b'.repeat(40))
    answers.set('svn_sync_status', () => ({
      ...syncStatus(IN_SYNC)(),
      headBehindBase: true,
    }))
    await changed()

    expect(count('svn_sync_status')).toBe(after + 1)
  })

  it('HEAD 没动：文件事件不跑第二档', async () => {
    mount()
    await settleOn(IN_SYNC)
    head('a'.repeat(40))
    await changed()
    const after = count('svn_sync_status')

    await changed()
    await changed()

    expect(count('svn_sync_status')).toBe(after)
  })

  it('校验失败：下一次事件重试', async () => {
    mount()
    await settleOn(IN_SYNC)
    head('a'.repeat(40))
    await changed()

    head('b'.repeat(40))
    answers.set('svn_sync_status', () => {
      throw { code: 'svn_failed', message: 'x' }
    })
    await changed()
    const after = count('svn_sync_status')

    answers.set('svn_sync_status', syncStatus(IN_SYNC))
    await changed()

    expect(count('svn_sync_status')).toBe(after + 1)
  })
})

describe('落后基线的分支', () => {
  it('文件事件之后重读', async () => {
    mount()
    await act(async () => {
      repoChanged.at(-1)?.({ payload: 'C:/r' })
      await new Promise((r) => setTimeout(r, 0))
    })
    expect(count('svn_branches_behind')).toBe(1)
  })

  it('别的仓库的文件事件不读', async () => {
    mount()
    await act(async () => {
      repoChanged.at(-1)?.({ payload: 'C:/other' })
      await new Promise((r) => setTimeout(r, 0))
    })
    expect(count('svn_branches_behind')).toBe(0)
  })
})
