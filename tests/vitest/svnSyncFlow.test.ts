import { beforeEach, describe, expect, it, vi } from 'vitest'

// 只替换发往后端的四个 invoke 包装；流程本身（先后顺序、清错、弹框）是被测对象
vi.mock('@/store/repoStore', () => ({
  svnSyncInPreview: vi.fn(),
  svnSyncIn: vi.fn(),
  svnSyncContinue: vi.fn(),
  svnSyncAbort: vi.fn(),
  svnFollow: vi.fn(),
  svnFollowAuto: vi.fn(),
  svnFollowContinue: vi.fn(),
  svnFollowManual: vi.fn(),
}))

import * as store from '@/store/repoStore'
import { createSvnSyncFlow, type SvnSyncFlowDeps } from '@/components/svnSyncFlow'
import type { ConfirmRequest } from '@/components/ConfirmDialog'
import type { SyncInPreview } from '@/engine/svnSyncIn'

const preview: SyncInPreview = {
  origBranch: 'work',
  origHead: 'h'.repeat(40),
  baselineOid: 'b'.repeat(40),
  baselineRev: 1,
  targetRev: 2,
  catchUpRev: null,
  outsideChanges: [],
}

/** 记下每一步的先后，断言「先清旧错误、再做事」。 */
function deps(log: string[], confirm?: (r: ConfirmRequest) => Promise<boolean>): SvnSyncFlowDeps {
  return {
    repoPath: 'C:/wc1',
    running: { current: false },
    syncState: () => null,
    guardMerge: async () => {
      log.push('guardMerge')
      return true
    },
    confirm:
      confirm ??
      (async (r) => {
        log.push(`confirm:${r.title}`)
        await r.action?.()
        return true
      }),
    clearError: () => log.push('clearError'),
    reportError: (err) => log.push(`reportError:${(err as { code?: string }).code ?? String(err)}`),
    refreshSvnSyncState: async () => {
      log.push('refreshSvnSyncState')
    },
    refreshSvnSync: async () => {
      log.push('refreshSvnSync')
    },
    fetchSvnLog: async () => {
      log.push('fetchSvnLog')
    },
    setFollowing: (busy) => log.push(`following:${busy}`),
    refreshAfterFollow: async () => {
      log.push('refreshAfterFollow')
    },
    showFollowReport: async () => {
      log.push('showFollowReport')
    },
    checkoutBranch: async (name) => {
      log.push(`checkout:${name}`)
    },
    mergeRiskForAuto: async () => {
      log.push('mergeRiskForAuto')
      return null
    },
    autoFollowEnabled: async () => true,
    wait: async (ms) => {
      log.push(`wait:${ms}`)
    },
    setAutoFollowSkip: (s) => log.push(`autoSkip:${s ? `${s.key}|${s.message}` : 'null'}`),
    showAutoFollowed: () => log.push('showAutoFollowed'),
    // 不记日志：上面那些按顺序断言的用例不该因为它变；要看它的用例自己覆盖
    clearAutoFollowed: () => {},
  }
}

/** 记下「清掉旧的已自动跟进提示」发生在哪一步之前。 */
function tracked(log: string[], confirm?: (r: ConfirmRequest) => Promise<boolean>) {
  return { ...deps(log, confirm), clearAutoFollowed: () => log.push('clearAutoFollowed') }
}

const report = {
  newFloor: 1,
  verified: true,
  changes: [],
  removed: [],
  dropped: [],
  rewritten: [],
  stage: { decisions: {}, uncertain: [], lost: [], discarded: [] },
  movedBranches: [],
  behindBranches: [],
  leaks: [],
  pinnedRef: null,
}

const conflict = {
  kind: 'needsManual' as const,
  at: 'c'.repeat(40),
  files: ['Code/A.cs'],
  baselinePublished: true,
  leaks: [],
  staleDisk: false,
}

beforeEach(() => {
  vi.mocked(store.svnSyncInPreview).mockReset()
  vi.mocked(store.svnSyncIn).mockReset()
  vi.mocked(store.svnSyncContinue).mockReset()
  vi.mocked(store.svnSyncAbort).mockReset()
  vi.mocked(store.svnFollow).mockReset()
  vi.mocked(store.svnFollowAuto).mockReset()
  vi.mocked(store.svnFollowContinue).mockReset()
  vi.mocked(store.svnFollowManual).mockReset()
})

describe('handleAutoFollow（事故分析 6.12）', () => {
  /** 自动跟进在后台跑完：等到按钮复位再断言。 */
  const settled = (log: string[]) => vi.waitFor(() => expect(log).toContain('following:false'))

  it('完成：不清错误条、不弹 merge driver 确认框，刷新之后挂提示，不弹报告框', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowAuto).mockImplementation(async () => {
      log.push('svnFollowAuto')
      return { kind: 'done', report }
    })

    expect(createSvnSyncFlow(deps(log)).handleAutoFollow('k1')).toBe(true)
    await settled(log)
    await vi.waitFor(() => expect(log).toContain('showAutoFollowed'))

    expect(store.svnFollowAuto).toHaveBeenCalledWith('C:/wc1')
    expect(log).toEqual([
      'mergeRiskForAuto',
      'following:true',
      'svnFollowAuto',
      'refreshAfterFollow',
      'refreshSvnSyncState',
      'refreshSvnSync',
      'fetchSvnLog',
      'following:false',
      'autoSkip:null',
      'showAutoFollowed',
    ])
  })

  it('不自动：原因按现场记给横幅，什么都不刷新、不挂提示', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowAuto).mockResolvedValue({
      kind: 'skipped',
      code: 'svn_auto_follow_conflict',
      message: '有冲突',
    })

    createSvnSyncFlow(deps(log)).handleAutoFollow('k1')
    await vi.waitFor(() => expect(log).toContain('autoSkip:k1|有冲突'))

    expect(log).not.toContain('refreshAfterFollow')
    expect(log).not.toContain('showAutoFollowed')
    expect(log).not.toContain('clearError')
  })

  it('设置里关掉了：不写原因，横幅照旧', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowAuto).mockResolvedValue({
      kind: 'skipped',
      code: 'svn_auto_follow_off',
      message: '设置里关掉了自动跟进。',
    })

    createSvnSyncFlow(deps(log)).handleAutoFollow('k1')
    await settled(log)
    await vi.waitFor(() => expect(log).toContain('autoSkip:null'))
    expect(log.filter((l) => l.startsWith('autoSkip:k1'))).toEqual([])
  })

  it('开关关着时先返回：不查 merge driver，也不写原因（review 中-2）', async () => {
    const log: string[] = []
    createSvnSyncFlow({
      ...deps(log),
      autoFollowEnabled: async () => false,
      mergeRiskForAuto: async () => {
        log.push('mergeRiskForAuto')
        return '没配 merge driver'
      },
    }).handleAutoFollow('k1')
    await vi.waitFor(() => expect(log).toContain('autoSkip:null'))

    expect(log).toEqual(['autoSkip:null'])
    expect(store.svnFollowAuto).not.toHaveBeenCalled()
  })

  it('暂时性原因（索引锁被占）等一下再试一次，成了就照常完成（review 中-1）', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowAuto)
      .mockResolvedValueOnce({ kind: 'skipped', code: 'svn_index_busy', message: '锁着' })
      .mockResolvedValueOnce({ kind: 'done', report })

    createSvnSyncFlow(deps(log)).handleAutoFollow('k1')
    await vi.waitFor(() => expect(log).toContain('showAutoFollowed'))

    expect(store.svnFollowAuto).toHaveBeenCalledTimes(2)
    expect(log).toContain('wait:2000')
    expect(log.filter((l) => l.startsWith('autoSkip:k1'))).toEqual([])
  })

  it('暂时性原因只重试一次，还不行就写进横幅；别的原因不重试', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowAuto).mockResolvedValue({
      kind: 'skipped',
      code: 'sync_in_progress',
      message: '在同步',
    })
    createSvnSyncFlow(deps(log)).handleAutoFollow('k1')
    await vi.waitFor(() => expect(log).toContain('autoSkip:k1|在同步'))
    expect(store.svnFollowAuto).toHaveBeenCalledTimes(2)

    vi.mocked(store.svnFollowAuto).mockClear()
    vi.mocked(store.svnFollowAuto).mockResolvedValue({
      kind: 'skipped',
      code: 'svn_auto_follow_conflict',
      message: '冲突',
    })
    const other: string[] = []
    createSvnSyncFlow(deps(other)).handleAutoFollow('k2')
    await vi.waitFor(() => expect(other).toContain('autoSkip:k2|冲突'))
    expect(store.svnFollowAuto).toHaveBeenCalledTimes(1)
    expect(other.some((l) => l.startsWith('wait:'))).toBe(false)
  })

  it('merge driver 有风险：不发命令、不问人，原因写进横幅', async () => {
    const log: string[] = []
    createSvnSyncFlow({
      ...deps(log),
      mergeRiskForAuto: async () => '没配 merge driver',
    }).handleAutoFollow('k1')
    await vi.waitFor(() => expect(log).toContain('autoSkip:k1|没配 merge driver'))

    expect(store.svnFollowAuto).not.toHaveBeenCalled()
    expect(log).not.toContain('guardMerge')
    expect(log).not.toContain('following:true')
  })

  it('有流程在走时不发起、返回 false；自动跟进在跑时横幅的「跟进」也不重入', async () => {
    const busy = deps([])
    busy.running.current = true
    expect(createSvnSyncFlow(busy).handleAutoFollow('k1')).toBe(false)
    expect(store.svnFollowAuto).not.toHaveBeenCalled()

    const log: string[] = []
    let release!: () => void
    vi.mocked(store.svnFollowAuto).mockImplementation(
      () =>
        new Promise((resolve) => {
          release = () => resolve({ kind: 'skipped', code: 'x', message: 'm' })
        }),
    )
    const d = deps(log)
    const flow = createSvnSyncFlow(d)
    flow.handleAutoFollow('k1')
    await vi.waitFor(() => expect(store.svnFollowAuto).toHaveBeenCalled())
    await flow.handleFollow()
    expect(store.svnFollow).not.toHaveBeenCalled()
    expect(flow.handleAutoFollow('k2')).toBe(false)
    release()
    await vi.waitFor(() => expect(d.running.current).toBe(false))
  })

  it('动过 ref 之后才失败：照常进错误条，不挂提示', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowAuto).mockRejectedValue({ code: 'svn_follow_index_rename_failed' })

    createSvnSyncFlow(deps(log)).handleAutoFollow('k1')
    await settled(log)

    expect(log).toContain('reportError:svn_follow_index_rename_failed')
    expect(log).not.toContain('showAutoFollowed')
  })
})

describe('新的一次跟进开始时清掉旧的「已自动跟进」提示（E2E 2026-11-13）', () => {
  // 旧提示说「放回了 1 个文件」，同时新横幅说「1 个文件缺 Local-Only 补丁」，两条挂在一起看着矛盾
  it('自动跟进：一开始就清，不自动时旧提示也不留', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowAuto).mockResolvedValue({
      kind: 'skipped',
      code: 'svn_auto_follow_conflict',
      message: '有冲突',
    })
    createSvnSyncFlow(tracked(log)).handleAutoFollow('k1')
    await vi.waitFor(() => expect(log).toContain('autoSkip:k1|有冲突'))
    expect(log[0]).toBe('clearAutoFollowed')
  })

  it('自动跟进完成：先清旧的，再挂新的', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowAuto).mockResolvedValue({ kind: 'done', report })
    createSvnSyncFlow(tracked(log)).handleAutoFollow('k1')
    await vi.waitFor(() => expect(log).toContain('showAutoFollowed'))
    expect(log.indexOf('clearAutoFollowed')).toBeLessThan(log.indexOf('showAutoFollowed'))
    expect(log.filter((l) => l === 'clearAutoFollowed')).toHaveLength(1)
  })

  it('横幅上的「跟进」：merge driver 确认之后、发命令之前清', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollow).mockImplementation(async () => {
      log.push('svnFollow')
      return { kind: 'done', report }
    })
    await createSvnSyncFlow(tracked(log)).handleFollow()
    const i = log.indexOf('clearAutoFollowed')
    expect(i).toBeGreaterThan(log.indexOf('guardMerge'))
    expect(i).toBeLessThan(log.indexOf('svnFollow'))
  })

  it('merge driver 没确认：什么都没发生，旧提示留着', async () => {
    const log: string[] = []
    await createSvnSyncFlow({ ...tracked(log), guardMerge: async () => false }).handleFollow()
    expect(log).not.toContain('clearAutoFollowed')
  })

  /** 确认框：记下弹出，再按 `yes` 决定执不执行 action。 */
  const answer = (log: string[], yes: boolean) => async (r: ConfirmRequest) => {
    log.push(`confirm:${r.title}`)
    if (yes) await r.action?.()
    return yes
  }

  it('「更新」：确认框弹出之后、执行时才清（更新完会再跟进，旧提示说的是更新前的事）', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncInPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncIn).mockImplementation(async () => {
      log.push('svnSyncIn')
      return { kind: 'done' } as never
    })
    await createSvnSyncFlow(tracked(log, answer(log, true))).handleSvnUpdate()
    const i = log.indexOf('clearAutoFollowed')
    expect(i).toBeGreaterThan(log.findIndex((l) => l.startsWith('confirm:')))
    expect(i).toBeLessThan(log.indexOf('svnSyncIn'))
  })

  it('「更新」确认框取消：不清', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncInPreview).mockResolvedValue(preview)
    await createSvnSyncFlow(tracked(log, answer(log, false))).handleSvnUpdate()
    expect(log.some((l) => l.startsWith('confirm:'))).toBe(true)
    expect(log).not.toContain('clearAutoFollowed')
  })

  it('跟进冲突转手动跟进：确认了清（跟进开始已清一次），取消不再清', async () => {
    vi.mocked(store.svnFollow).mockResolvedValue(conflict)
    vi.mocked(store.svnFollowManual).mockResolvedValue({ kind: 'done' } as never)
    const yes: string[] = []
    await createSvnSyncFlow(tracked(yes, answer(yes, true))).handleFollow()
    expect(store.svnFollowManual).toHaveBeenCalled()
    expect(yes.filter((l) => l === 'clearAutoFollowed')).toHaveLength(2)

    vi.mocked(store.svnFollowManual).mockClear()
    const no: string[] = []
    await createSvnSyncFlow(tracked(no, answer(no, false))).handleFollow()
    expect(store.svnFollowManual).not.toHaveBeenCalled()
    expect(no.filter((l) => l === 'clearAutoFollowed')).toHaveLength(1)
  })

  it('跟进记录的「继续」：确认了清，取消不清', async () => {
    vi.mocked(store.svnFollowContinue).mockResolvedValue({ kind: 'done', report })
    const yes: string[] = []
    await createSvnSyncFlow(tracked(yes, answer(yes, true))).handleSyncAction('followContinue')
    expect(store.svnFollowContinue).toHaveBeenCalled()
    expect(yes).toContain('clearAutoFollowed')

    const no: string[] = []
    await createSvnSyncFlow(tracked(no, answer(no, false))).handleSyncAction('followContinue')
    expect(no).not.toContain('clearAutoFollowed')
  })

  it('更新记录的继续 / 放弃：不是新的跟进，不清', async () => {
    vi.mocked(store.svnSyncContinue).mockResolvedValue({ kind: 'done' } as never)
    vi.mocked(store.svnSyncAbort).mockResolvedValue({ kind: 'done' } as never)
    for (const id of ['syncContinue', 'syncAbort'] as const) {
      const log: string[] = []
      await createSvnSyncFlow(tracked(log, answer(log, true))).handleSyncAction(id)
      expect(log.some((l) => l.startsWith('confirm:'))).toBe(true)
      expect(log).not.toContain('clearAutoFollowed')
    }
  })

  it('自动跟进开关关着、merge driver 有风险：也清（又来了新的变化，旧提示过时；要留意的报告由 keepOnNewFollow 留住）', async () => {
    const off: string[] = []
    createSvnSyncFlow({ ...tracked(off), autoFollowEnabled: async () => false }).handleAutoFollow('k1')
    await vi.waitFor(() => expect(off).toContain('autoSkip:null'))
    expect(off[0]).toBe('clearAutoFollowed')

    const risky: string[] = []
    createSvnSyncFlow({ ...tracked(risky), mergeRiskForAuto: async () => '没配' }).handleAutoFollow('k1')
    await vi.waitFor(() => expect(risky).toContain('autoSkip:k1|没配'))
    expect(risky[0]).toBe('clearAutoFollowed')
    expect(store.svnFollowAuto).not.toHaveBeenCalled()
  })

  it('有流程在走时不发起，也不清', async () => {
    const log: string[] = []
    const d = tracked(log)
    d.running.current = true
    expect(createSvnSyncFlow(d).handleAutoFollow('k1')).toBe(false)
    await createSvnSyncFlow(d).handleFollow()
    expect(log).not.toContain('clearAutoFollowed')
  })
})

describe('handleFollow', () => {
  it('banner follow button invokes svnFollow：执行中按钮禁用，完成后刷新再弹报告', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollow).mockImplementation(async () => {
      log.push('svnFollow')
      return { kind: 'done', report }
    })

    await createSvnSyncFlow(deps(log)).handleFollow()

    expect(store.svnFollow).toHaveBeenCalledWith('C:/wc1')
    expect(log).toEqual([
      'clearError',
      'guardMerge',
      'following:true',
      'svnFollow',
      'refreshAfterFollow',
      'refreshSvnSyncState',
      'refreshSvnSync',
      'fetchSvnLog',
      'following:false',
      'showFollowReport',
    ])
  })

  it('报告拿到的是后端给的那一份', async () => {
    vi.mocked(store.svnFollow).mockResolvedValue({ kind: 'done', report })
    const shown: unknown[] = []
    await createSvnSyncFlow({
      ...deps([]),
      showFollowReport: async (r) => {
        shown.push(r)
      },
    }).handleFollow()
    expect(shown).toEqual([report])
  })

  it('merge driver 没确认时什么都不发，按钮也不动', async () => {
    const log: string[] = []
    await createSvnSyncFlow({ ...deps(log), guardMerge: async () => false }).handleFollow()
    expect(store.svnFollow).not.toHaveBeenCalled()
    expect(log).toEqual(['clearError'])
  })

  it('同一时间只走一个流程，不会发第二次', async () => {
    let release!: (o: { kind: 'done'; report: typeof report }) => void
    vi.mocked(store.svnFollow).mockReturnValue(
      new Promise((r) => {
        release = r
      }),
    )
    const d = deps([])

    const first = createSvnSyncFlow(d).handleFollow()
    await vi.waitFor(() => expect(store.svnFollow).toHaveBeenCalled())
    await createSvnSyncFlow(d).handleFollow()
    await createSvnSyncFlow(d).handleSvnUpdate()
    release({ kind: 'done', report })
    await first

    expect(store.svnFollow).toHaveBeenCalledTimes(1)
    expect(store.svnSyncInPreview).not.toHaveBeenCalled()
  })

  it('拒绝：显示错误，不弹报告，按钮复位', async () => {
    const log: string[] = []
    const seen: ConfirmRequest[] = []
    vi.mocked(store.svnFollow).mockResolvedValue({
      kind: 'refused',
      code: 'svn_follow_git_busy',
      message: 'git 正在 rebase',
    })

    await createSvnSyncFlow(
      deps(log, async (r) => {
        seen.push(r)
        return true
      }),
    ).handleFollow()

    expect(seen).toHaveLength(1)
    expect(seen[0].notice).toBe('attention')
    expect(seen[0].message).toContain('git 正在 rebase')
    expect(log).not.toContain('showFollowReport')
    expect(log).toContain('following:false')
  })

  it('命令本身抛错：进错误条，按钮复位', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollow).mockRejectedValue({ code: 'io', message: 'x' })

    await createSvnSyncFlow(deps(log)).handleFollow()

    expect(log).toContain('reportError:io')
    expect(log.at(-1)).toBe('following:false')
    expect(log).not.toContain('showFollowReport')
  })

  it('刷新失败不盖掉结局：跟进已经落地，报告照弹', async () => {
    const log: string[] = []
    const d = deps(log)
    d.refreshAfterFollow = async () => {
      throw { code: 'load_graph_failed' }
    }
    vi.mocked(store.svnFollow).mockResolvedValue({ kind: 'done', report })

    await createSvnSyncFlow(d).handleFollow()

    expect(log).toContain('reportError:load_graph_failed')
    expect(log.at(-1)).toBe('showFollowReport')
  })

  it('冲突：确认之后才走手动跟进，确认框上有冲突的文件，不再过一遍 merge driver', async () => {
    const log: string[] = []
    const seen: ConfirmRequest[] = []
    vi.mocked(store.svnFollow).mockResolvedValue(conflict)
    vi.mocked(store.svnFollowManual).mockImplementation(async () => {
      log.push('followManual')
      return { kind: 'done', newRev: 2 }
    })

    await createSvnSyncFlow(
      deps(log, async (r) => {
        seen.push(r)
        await r.action?.()
        return true
      }),
    ).handleFollow()

    expect(seen[0].title).toBe('跟进遇到冲突')
    expect(seen[0].message).toContain('Code/A.cs')
    expect(seen[0].confirmLabel).toBe('手动跟进')
    expect(store.svnFollowManual).toHaveBeenCalledWith('C:/wc1')
    expect(log.filter((l) => l === 'guardMerge')).toHaveLength(1)
    expect(log).not.toContain('showFollowReport')
  })

  it('冲突时取消：手动跟进永远不自动发起', async () => {
    vi.mocked(store.svnFollow).mockResolvedValue(conflict)

    await createSvnSyncFlow(deps([], async () => false)).handleFollow()

    expect(store.svnFollowManual).not.toHaveBeenCalled()
  })

  it('手动跟进遇到冲突：后续的继续 / 放弃走横幅，这里把结局报出来', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollow).mockResolvedValue(conflict)
    vi.mocked(store.svnFollowManual).mockResolvedValue({ kind: 'rebaseConflict', files: ['A'] })

    await createSvnSyncFlow(deps(log)).handleFollow()

    expect(log.at(-1)).toBe('confirm:手动跟进遇到冲突')
  })
})

describe('handleCheckoutBehind', () => {
  /** 按下确认框上的哪一颗按钮。 */
  const press =
    (log: string[], pick: 'confirm' | 'alternative' | 'cancel', seen: ConfirmRequest[] = []) =>
    async (r: ConfirmRequest): Promise<boolean> => {
      seen.push(r)
      log.push(`confirm:${r.title}`)
      // 第二个框是手动跟进自己的确认
      if (seen.length > 1 || pick === 'confirm') {
        await r.action?.()
        return true
      }
      if (pick === 'alternative') r.alternative?.onSelect()
      return false
    }

  it('切换并手动跟进（默认）：切分支 → merge driver → 手动跟进自己的确认框', async () => {
    const log: string[] = []
    const seen: ConfirmRequest[] = []
    vi.mocked(store.svnFollowManual).mockImplementation(async () => {
      log.push('followManual')
      return { kind: 'done', newRev: 2 }
    })

    await createSvnSyncFlow(deps(log, press(log, 'confirm', seen))).handleCheckoutBehind('old')

    expect(seen[0].confirmLabel).toBe('切换并手动跟进')
    expect(seen[0].alternative?.label).toBe('仅切换')
    expect(log).toEqual([
      'clearError',
      'confirm:切换到落后基线的分支',
      'checkout:old',
      'clearError',
      'guardMerge',
      'confirm:手动跟进',
      'followManual',
      'refreshSvnSyncState',
      'refreshSvnSync',
      'fetchSvnLog',
    ])
  })

  it('仅切换：只切分支，不碰手动跟进', async () => {
    const log: string[] = []
    await createSvnSyncFlow(deps(log, press(log, 'alternative'))).handleCheckoutBehind('old')
    expect(log).toContain('checkout:old')
    expect(store.svnFollowManual).not.toHaveBeenCalled()
    expect(log).not.toContain('guardMerge')
  })

  it('取消：什么都不做', async () => {
    const log: string[] = []
    await createSvnSyncFlow(deps(log, press(log, 'cancel'))).handleCheckoutBehind('old')
    expect(log).not.toContain('checkout:old')
    expect(store.svnFollowManual).not.toHaveBeenCalled()
  })

  it('切分支被工作区改动挡住：沿用错误提示，不走手动跟进，也不替用户 stash', async () => {
    const log: string[] = []
    const d = deps(log, press(log, 'confirm'))
    d.checkoutBranch = async () => {
      throw { code: 'checkout_blocked', message: 'local changes would be overwritten' }
    }

    await createSvnSyncFlow(d).handleCheckoutBehind('old')

    expect(log).toContain('reportError:checkout_blocked')
    expect(store.svnFollowManual).not.toHaveBeenCalled()
    expect(log).not.toContain('confirm:手动跟进')
  })

  it('同一时间只走一个流程', async () => {
    const log: string[] = []
    const d = deps(log)
    d.running.current = true
    await createSvnSyncFlow(d).handleCheckoutBehind('old')
    expect(log).toEqual([])
  })
})

describe('切换之后的手动跟进（只经由「切换并手动跟进」发起，Review 3 第 4 条）', () => {
  /** 切分支的框点确认（它没有 action）；之后的框交给 `rest`。 */
  const afterSwitch =
    (log: string[], rest: (r: ConfirmRequest) => Promise<boolean>) =>
    async (r: ConfirmRequest): Promise<boolean> => {
      if (r.title === '切换到落后基线的分支') {
        log.push(`confirm:${r.title}`)
        return true
      }
      return rest(r)
    }
  const accept = (log: string[]) => async (r: ConfirmRequest) => {
    log.push(`confirm:${r.title}`)
    await r.action?.()
    return true
  }

  it('永远不自动执行：手动跟进的框不确认就什么都不发', async () => {
    const log: string[] = []
    const decline = async (r: ConfirmRequest) => {
      log.push(`confirm:${r.title}`)
      return false
    }
    await createSvnSyncFlow(deps(log, afterSwitch(log, decline))).handleCheckoutBehind('old')
    expect(log).toContain('confirm:手动跟进')
    expect(store.svnFollowManual).not.toHaveBeenCalled()
    expect(log).not.toContain('refreshSvnSync')
  })

  it('merge driver 没确认时连手动跟进的框都不弹', async () => {
    const log: string[] = []
    await createSvnSyncFlow({
      ...deps(log, afterSwitch(log, accept(log))),
      guardMerge: async () => false,
    }).handleCheckoutBehind('old')
    expect(store.svnFollowManual).not.toHaveBeenCalled()
    expect(log).toEqual([
      'clearError',
      'confirm:切换到落后基线的分支',
      'checkout:old',
      'clearError',
    ])
  })

  it('冲突出口与暂存没保住都要让用户看见', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowManual).mockResolvedValueOnce({ kind: 'rebaseConflict', files: ['A'] })
    await createSvnSyncFlow(deps(log, afterSwitch(log, accept(log)))).handleCheckoutBehind('old')
    expect(log.at(-1)).toBe('confirm:手动跟进遇到冲突')

    vi.mocked(store.svnFollowManual).mockResolvedValueOnce({
      kind: 'stageLost',
      newRev: 2,
      stashOid: 's'.repeat(40),
      files: ['Code/B.cs'],
    })
    await createSvnSyncFlow(deps(log, afterSwitch(log, accept(log)))).handleCheckoutBehind('old')
    expect(log.at(-1)).toBe('confirm:手动跟进完成，暂存状态没保住')
  })
})

describe('横幅上手动跟进记录的继续 / 放弃', () => {
  const unfinished = (phase: 'rebase' | 'applying-stash') => () => ({
    locked: false,
    holder: null,
    unfinished: { flow: 'follow_manual' as const, phase, opid: 'x', startedAt: 1 },
  })

  it('走更新的继续命令，文案说手动跟进', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncContinue).mockResolvedValue({ kind: 'done', newRev: 2 })

    await createSvnSyncFlow({ ...deps(log), syncState: unfinished('rebase') }).handleSyncAction(
      'syncContinue',
    )

    expect(store.svnSyncContinue).toHaveBeenCalledWith('C:/wc1')
    expect(store.svnFollowContinue).not.toHaveBeenCalled()
    expect(log).toContain('confirm:继续手动跟进')
  })

  it('放弃的结局也说手动跟进', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncAbort).mockResolvedValue({ kind: 'aborted' })

    await createSvnSyncFlow({ ...deps(log), syncState: unfinished('rebase') }).handleSyncAction(
      'syncAbort',
    )

    expect(log).toContain('confirm:放弃这次手动跟进')
    expect(log.at(-1)).toBe('confirm:已放弃这次手动跟进')
  })

  it('继续到最后暂存没保住：手动跟进的记录按手动跟进的话报出来', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncContinue).mockResolvedValue({
      kind: 'stageLost',
      newRev: 2,
      stashOid: 's'.repeat(40),
      files: [],
    })

    await createSvnSyncFlow({
      ...deps(log),
      syncState: unfinished('applying-stash'),
    }).handleSyncAction('syncContinue')

    expect(log.at(-1)).toBe('confirm:手动跟进完成，暂存状态没保住')
  })
})

describe('handleSvnUpdate', () => {
  it('重新点「更新」先清掉上一次的错误条，再去预览（2026-09-30 E2E 场景 6 → 7）', async () => {
    // 场景 6 的「分支上有 merge」红条在用户撤掉 merge 之后仍挂着，与新横幅叠在一起
    const log: string[] = []
    vi.mocked(store.svnSyncInPreview).mockImplementation(async () => {
      log.push('preview')
      return preview
    })
    vi.mocked(store.svnSyncIn).mockResolvedValue({ kind: 'done', newRev: 2 })

    await createSvnSyncFlow(deps(log)).handleSvnUpdate()

    expect(log.indexOf('clearError')).toBeGreaterThanOrEqual(0)
    expect(log.indexOf('clearError')).toBeLessThan(log.indexOf('preview'))
  })

  it('这一次预览又被拒：先清旧的、再报新的——新错误不能被清掉', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncInPreview).mockRejectedValue({ code: 'sync_in_merge_in_range', message: 'm' })

    await createSvnSyncFlow(deps(log)).handleSvnUpdate()

    expect(log).toEqual(['clearError', 'guardMerge', 'reportError:sync_in_merge_in_range'])
    expect(store.svnSyncIn).not.toHaveBeenCalled()
  })

  it('确认之后执行、刷新 SVN 侧状态；冲突结局再弹一个「知道了」', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncInPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncIn).mockResolvedValue({ kind: 'rebaseConflict', files: ['Code/A.cs'] })
    const titles: (string | undefined)[] = []
    const confirm = async (r: ConfirmRequest) => {
      titles.push(r.confirmLabel)
      await r.action?.()
      return true
    }

    await createSvnSyncFlow(deps(log, confirm)).handleSvnUpdate()

    expect(store.svnSyncIn).toHaveBeenCalledWith('C:/wc1', {
      origBranch: 'work',
      origHead: preview.origHead,
      baselineOid: preview.baselineOid,
      confirmedOutside: [],
    })
    expect(log).toContain('refreshSvnSyncState')
    expect(titles).toEqual(['更新', '知道了'])
  })

  it('结局提示以 notice 弹出（只有一个按钮），确认框本身不是 notice', async () => {
    vi.mocked(store.svnSyncInPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncIn).mockResolvedValue({ kind: 'rebaseConflict', files: ['Code/A.cs'] })
    const notices: (string | undefined)[] = []
    const confirm = async (r: ConfirmRequest) => {
      notices.push(r.notice)
      await r.action?.()
      return true
    }

    await createSvnSyncFlow(deps([], confirm)).handleSvnUpdate()

    expect(notices).toEqual([undefined, 'attention'])
  })
})

describe('同一时间只走一个流程（PR #49 Codex review #5）', () => {
  it('预览还在路上时再点「更新」或横幅按钮：不再发第二次', async () => {
    // 预览要跑 `svn status -u`（网络），几秒里界面上没有框挡着，工具栏和横幅都点得到
    let release!: (p: SyncInPreview) => void
    vi.mocked(store.svnSyncInPreview).mockReturnValue(
      new Promise((r) => {
        release = r
      }),
    )
    vi.mocked(store.svnSyncIn).mockResolvedValue({ kind: 'done', newRev: 2 })
    // RepoTab 每次渲染都重新 create 一次：标记必须活在注入的 `running` 里，不能在闭包里
    const d = deps([])

    const first = createSvnSyncFlow(d).handleSvnUpdate()
    await createSvnSyncFlow(d).handleSvnUpdate()
    await createSvnSyncFlow(d).handleSyncAction('syncAbort')
    release(preview)
    await first

    expect(store.svnSyncInPreview).toHaveBeenCalledTimes(1)
    expect(store.svnSyncAbort).not.toHaveBeenCalled()
    expect(store.svnSyncIn).toHaveBeenCalledTimes(1)
  })

  it('上一次走完（包括失败）之后可以再点', async () => {
    vi.mocked(store.svnSyncInPreview).mockRejectedValueOnce({ code: 'x', message: 'm' })
    const flow = createSvnSyncFlow(deps([]))
    await flow.handleSvnUpdate()
    vi.mocked(store.svnSyncInPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncIn).mockResolvedValue({ kind: 'done', newRev: 2 })
    await flow.handleSvnUpdate()
    expect(store.svnSyncInPreview).toHaveBeenCalledTimes(2)
    expect(store.svnSyncIn).toHaveBeenCalledTimes(1)
  })
})

describe('收尾之后的刷新失败（3a 最终 review 第 3 条在「更新」侧的同形问题）', () => {
  /** 与真对话框一样：`action` 抛错时留在原地显示错误。 */
  const dialog =
    (log: string[]) =>
    async (r: ConfirmRequest): Promise<boolean> => {
      log.push(`confirm:${r.title}`)
      try {
        await r.action?.()
      } catch (e) {
        log.push(`dialogError:${(e as { code?: string }).code}`)
        return false
      }
      return true
    }

  it('更新成功后刷新失败：对话框不报错，刷新的错误进错误条', async () => {
    // 更新已经落地，对话框却留在原地报错、「更新」还能点，用户会以为没更新上
    const log: string[] = []
    const d = deps(log, dialog(log))
    d.refreshSvnSync = async () => {
      throw { code: 'svn_check_failed' }
    }
    vi.mocked(store.svnSyncInPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncIn).mockResolvedValue({ kind: 'rebaseConflict', files: ['Code/A.cs'] })

    await createSvnSyncFlow(d).handleSvnUpdate()

    expect(log.some((l) => l.startsWith('dialogError'))).toBe(false)
    expect(log).toContain('reportError:svn_check_failed')
    // 结局照常弹出
    expect(log.filter((l) => l.startsWith('confirm:'))).toHaveLength(2)
    expect(log).toContain('fetchSvnLog')
  })

  it('横幅上的继续成功后刷新失败：同样不报到对话框里', async () => {
    const log: string[] = []
    const d = deps(log, dialog(log))
    d.refreshSvnSyncState = async () => {
      throw { code: 'journal_unreadable' }
    }
    vi.mocked(store.svnSyncContinue).mockResolvedValue({ kind: 'done', newRev: 2 })

    await createSvnSyncFlow(d).handleSyncAction('syncContinue')

    expect(log.some((l) => l.startsWith('dialogError'))).toBe(false)
    expect(log).toContain('reportError:journal_unreadable')
  })

  it('执行本身失败：对话框里显示的仍是后端原本的错误', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncInPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncIn).mockRejectedValue({ code: 'sync_in_update_failed' })

    await createSvnSyncFlow(deps(log, dialog(log))).handleSvnUpdate()

    expect(log).toContain('dialogError:sync_in_update_failed')
  })
})

describe('handleSyncAction', () => {
  it('横幅上的继续 / 放弃同样先清掉上一次的错误条', async () => {
    for (const id of ['syncContinue', 'syncAbort'] as const) {
      const log: string[] = []
      vi.mocked(store.svnSyncContinue).mockImplementation(async () => {
        log.push('continue')
        return { kind: 'done', newRev: 2 }
      })
      vi.mocked(store.svnSyncAbort).mockImplementation(async () => {
        log.push('abort')
        return { kind: 'aborted' }
      })

      await createSvnSyncFlow(deps(log)).handleSyncAction(id)

      const act = id === 'syncContinue' ? 'continue' : 'abort'
      expect(log.indexOf('clearError'), id).toBeGreaterThanOrEqual(0)
      expect(log.indexOf('clearError'), id).toBeLessThan(log.indexOf(act))
    }
  })
})

describe('跟进记录的「继续」', () => {
  it('清错 → merge driver 确认 → 执行 → 刷新 → 弹报告，走的是跟进的命令', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowContinue).mockImplementation(async () => {
      log.push('followContinue')
      return { kind: 'done', report }
    })

    await createSvnSyncFlow(deps(log)).handleSyncAction('followContinue')

    expect(store.svnSyncContinue).not.toHaveBeenCalled()
    expect(store.svnSyncAbort).not.toHaveBeenCalled()
    expect(log).toEqual([
      'clearError',
      'guardMerge',
      'confirm:继续跟进',
      'followContinue',
      'refreshAfterFollow',
      'refreshSvnSyncState',
      'refreshSvnSync',
      'fetchSvnLog',
      'showFollowReport',
    ])
  })

  it('恢复之后重新跟进遇到冲突：同样落到手动跟进，不留一句没有按钮的话', async () => {
    const log: string[] = []
    vi.mocked(store.svnFollowContinue).mockResolvedValue(conflict)
    vi.mocked(store.svnFollowManual).mockResolvedValue({ kind: 'done', newRev: 2 })

    await createSvnSyncFlow(deps(log)).handleSyncAction('followContinue')

    expect(log).toContain('confirm:跟进遇到冲突')
    expect(store.svnFollowManual).toHaveBeenCalledTimes(1)
  })

  it('恢复被拒：把后端的话告诉用户', async () => {
    const seen: ConfirmRequest[] = []
    vi.mocked(store.svnFollowContinue).mockResolvedValue({
      kind: 'refused',
      code: 'svn_index_busy',
      message: '索引被占着',
    })

    await createSvnSyncFlow(
      deps([], async (r) => {
        seen.push(r)
        await r.action?.()
        return true
      }),
    ).handleSyncAction('followContinue')

    expect(seen.at(-1)?.message).toContain('索引被占着')
    expect(seen.at(-1)?.notice).toBe('attention')
  })

  it('恢复可能重新重放，guardMerge 返回 false 时什么都不做', async () => {
    const log: string[] = []
    await createSvnSyncFlow({ ...deps(log), guardMerge: async () => false }).handleSyncAction(
      'followContinue',
    )
    expect(store.svnFollowContinue).not.toHaveBeenCalled()
  })

  it('取消确认框时没有结局可报', async () => {
    const log: string[] = []
    await createSvnSyncFlow(deps(log, async () => false)).handleSyncAction('followContinue')
    expect(store.svnFollowContinue).not.toHaveBeenCalled()
    expect(log).not.toContain('refreshSvnSync')
  })
})

describe('合并前的 merge driver 确认', () => {
  it('guardMerge 返回 false 时「更新」不预览、不执行', async () => {
    const log: string[] = []
    await createSvnSyncFlow({ ...deps(log), guardMerge: async () => false }).handleSvnUpdate()
    expect(store.svnSyncInPreview).not.toHaveBeenCalled()
    expect(store.svnSyncIn).not.toHaveBeenCalled()
  })

  it('「更新」先过 guardMerge 再预览', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncInPreview).mockImplementation(async () => {
      log.push('preview')
      return preview
    })
    vi.mocked(store.svnSyncIn).mockResolvedValue({ kind: 'done', newRev: 2 })
    await createSvnSyncFlow(deps(log)).handleSvnUpdate()
    expect(log.indexOf('guardMerge')).toBeGreaterThanOrEqual(0)
    expect(log.indexOf('guardMerge')).toBeLessThan(log.indexOf('preview'))
  })

  it('横幅上的「继续」要过 guardMerge，返回 false 时不调后端', async () => {
    const log: string[] = []
    await createSvnSyncFlow({ ...deps(log), guardMerge: async () => false }).handleSyncAction('syncContinue')
    expect(store.svnSyncContinue).not.toHaveBeenCalled()
  })

  it('「放弃」不合并，不过 guardMerge', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncAbort).mockResolvedValue({ kind: 'aborted' } as never)
    await createSvnSyncFlow({ ...deps(log), guardMerge: async () => false }).handleSyncAction('syncAbort')
    expect(store.svnSyncAbort).toHaveBeenCalled()
  })
})
