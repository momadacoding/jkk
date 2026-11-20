import { beforeEach, describe, expect, it, vi } from 'vitest'

// 只替换发往后端的四个 invoke 包装；流程本身（先后顺序、清错、草稿、弹框）是被测对象
vi.mock('@/store/repoStore', () => ({
  svnSyncOutPreview: vi.fn(),
  svnSyncOut: vi.fn(),
  svnSyncOutContinue: vi.fn(),
  svnSyncOutAbort: vi.fn(),
}))

import * as store from '@/store/repoStore'
import {
  createSvnSyncOutFlow,
  type DeliverRequest,
  type SvnSyncOutFlowDeps,
} from '@/components/svnSyncOutFlow'
import type { ConfirmRequest } from '@/components/ConfirmDialog'
import type { SyncOutPreview } from '@/engine/svnSyncOut'

const preview: SyncOutPreview = {
  origBranch: 'work',
  origHead: 'h'.repeat(40),
  baselineOid: 'b'.repeat(40),
  baselineRev: 1,
  remoteRev: 1,
  deliverOid: 'd'.repeat(40),
  deliverTree: 't'.repeat(40),
  changes: [{ status: 'M', path: 'Code/A.cs' }],
  localOnly: [],
  outsideChanges: [],
}

/** 模拟两个对话框：`action` 抛错时与真对话框一样留在原地显示错误，之后用户关掉。 */
function deps(log: string[], typed = '修复跳跃'): SvnSyncOutFlowDeps & { draft: { current: string } } {
  const draft = { current: '' }
  return {
    repoPath: 'C:/wc1',
    running: { current: false },
    draft,
    syncState: () => null,
    guardMerge: async () => {
      log.push('guardMerge')
      return true
    },
    deliver: async (r: DeliverRequest) => {
      log.push(`deliver:initial=${r.initialMessage}`)
      r.onMessageChange(typed)
      try {
        await r.action(typed)
      } catch (e) {
        log.push(`dialogError:${(e as { code?: string }).code}`)
        return false
      }
      return true
    },
    confirm: async (r: ConfirmRequest) => {
      log.push(`confirm:${r.title}`)
      try {
        await r.action?.()
      } catch (e) {
        log.push(`dialogError:${(e as { code?: string }).code}`)
        return false
      }
      return true
    },
    clearError: () => log.push('clearError'),
    reportError: (err) => log.push(`reportError:${(err as { code?: string }).code}`),
    refreshSvnSyncState: async () => {
      log.push('refreshSvnSyncState')
    },
    refreshSvnSync: async () => {
      log.push('refreshSvnSync')
    },
    fetchSvnLog: async () => {
      log.push('fetchSvnLog')
    },
    // 不记日志：只有补丁落回阶段的「继续」用它，那几个用例自己接
    setSyncActionBusy: () => {},
  }
}

beforeEach(() => {
  for (const f of [store.svnSyncOutPreview, store.svnSyncOut, store.svnSyncOutContinue, store.svnSyncOutAbort]) {
    vi.mocked(f).mockReset()
  }
})

describe('handleSvnDeliver', () => {
  it('先清旧错误再预览；预览被拒直接报错，不弹对话框', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncOutPreview).mockRejectedValue({ code: 'sync_out_behind_baseline' })
    await createSvnSyncOutFlow(deps(log)).handleSvnDeliver()
    expect(log).toEqual(['clearError', 'guardMerge', 'reportError:sync_out_behind_baseline'])
  })

  it('成功：带着用户写的信息执行、刷新、清空草稿、弹 D9 提示', async () => {
    const log: string[] = []
    const d = deps(log)
    vi.mocked(store.svnSyncOutPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncOut).mockResolvedValue({ kind: 'delivered', rev: 2, branch: 'work', localOnlyLeft: 0 })
    await createSvnSyncOutFlow(d).handleSvnDeliver()
    expect(vi.mocked(store.svnSyncOut).mock.calls[0][1].message).toBe('修复跳跃')
    expect(log).toContain('refreshSvnSyncState')
    expect(d.draft.current).toBe('')
    expect(log.at(-1)).toBe('confirm:已交付 r2')
  })

  it('执行失败（比如服务器在确认期间前进了）：刷新状态、保留提交信息（spec §13）', async () => {
    const log: string[] = []
    const d = deps(log)
    vi.mocked(store.svnSyncOutPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncOut).mockRejectedValue({ code: 'sync_out_server_ahead' })
    await createSvnSyncOutFlow(d).handleSvnDeliver()
    expect(log).toContain('dialogError:sync_out_server_ahead')
    expect(log).toContain('refreshSvnSyncState')
    expect(d.draft.current).toBe('修复跳跃')

    // 下一次打开对话框，草稿原样带回来
    const log2: string[] = []
    const d2 = { ...deps(log2), draft: d.draft }
    vi.mocked(store.svnSyncOut).mockResolvedValue({ kind: 'delivered', rev: 3, branch: 'work', localOnlyLeft: 0 })
    await createSvnSyncOutFlow(d2).handleSvnDeliver()
    expect(log2).toContain('deliver:initial=修复跳跃')
  })

  it('交付成功后刷新失败：结局照常弹出，刷新的错误进错误条，不能让对话框以为交付失败了', async () => {
    // 最终 review 第 3 条：提交已在服务器上，对话框却留在原地报错、「交付」还能点
    const log: string[] = []
    const d = deps(log)
    d.refreshSvnSync = async () => {
      throw { code: 'svn_check_failed' }
    }
    vi.mocked(store.svnSyncOutPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncOut).mockResolvedValue({ kind: 'delivered', rev: 2, branch: 'work', localOnlyLeft: 0 })
    await createSvnSyncOutFlow(d).handleSvnDeliver()
    expect(log.some((l) => l.startsWith('dialogError'))).toBe(false)
    expect(log).toContain('reportError:svn_check_failed')
    expect(log).toContain('confirm:已交付 r2')
    expect(d.draft.current).toBe('')
  })

  it('执行失败后刷新也失败：对话框里显示的仍是后端原本的错误', async () => {
    // D6 失败的原文是用户唯一知道「提交已经成功、点继续」的地方，不能被刷新错误顶掉
    const log: string[] = []
    const d = deps(log)
    d.refreshSvnSync = async () => {
      throw { code: 'svn_check_failed' }
    }
    vi.mocked(store.svnSyncOutPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncOut).mockRejectedValue({ code: 'sync_out_update_failed' })
    await createSvnSyncOutFlow(d).handleSvnDeliver()
    expect(log).toContain('dialogError:sync_out_update_failed')
    expect(log).not.toContain('dialogError:svn_check_failed')
  })

  it('结局提示以 notice 弹出：交付成功是 ok，只有一个按钮', async () => {
    const d = deps([])
    const notices: (string | undefined)[] = []
    d.confirm = async (r: ConfirmRequest) => {
      notices.push(r.notice)
      return true
    }
    vi.mocked(store.svnSyncOutPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncOut).mockResolvedValue({ kind: 'delivered', rev: 2, branch: 'work', localOnlyLeft: 0 })
    await createSvnSyncOutFlow(d).handleSvnDeliver()
    expect(notices).toEqual(['ok'])
  })

  it('横幅按钮：继续成功后刷新失败，结局照常弹出', async () => {
    const log: string[] = []
    const d = deps(log)
    d.refreshSvnSyncState = async () => {
      throw { code: 'journal_unreadable' }
    }
    vi.mocked(store.svnSyncOutContinue).mockResolvedValue({ kind: 'delivered', rev: 2, branch: 'work', localOnlyLeft: 0 })
    await createSvnSyncOutFlow(d).handleSyncOutAction('syncOutContinue')
    expect(log.some((l) => l.startsWith('dialogError'))).toBe(false)
    expect(log).toContain('reportError:journal_unreadable')
    expect(log).toContain('confirm:已交付 r2')
    // 同步状态读不出来不连累失同步的复查：两项同时发（2026-11-14，同 svnSyncFlow）
    expect(log).toContain('refreshSvnSync')
  })

  it('同一时间只走一个流程', async () => {
    const log: string[] = []
    const d = deps(log)
    d.running.current = true
    await createSvnSyncOutFlow(d).handleSvnDeliver()
    expect(log).toEqual([])
  })
})

describe('handleSyncOutAction', () => {
  it('「清理」走放弃那个命令；「继续」走继续', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncOutAbort).mockResolvedValue({ kind: 'abandoned', committed: false })
    vi.mocked(store.svnSyncOutContinue).mockResolvedValue({ kind: 'delivered', rev: 2, branch: 'work', localOnlyLeft: 0 })
    const flow = createSvnSyncOutFlow(deps(log))
    await flow.handleSyncOutAction('syncOutCleanup')
    expect(store.svnSyncOutAbort).toHaveBeenCalledTimes(1)
    await flow.handleSyncOutAction('syncOutContinue')
    expect(store.svnSyncOutContinue).toHaveBeenCalledTimes(1)
  })

  it('继续失败（比如 D6 又断网）：横幅状态也要刷新，错误留在对话框里', async () => {
    // 失败也可能推进了 journal（D6 之后停在 D7a、或从 post-commit 走到 publishing-baseline），
    // 不刷新的话横幅停在旧 phase，按钮与文案都是错的
    const log: string[] = []
    vi.mocked(store.svnSyncOutContinue).mockRejectedValue({ code: 'sync_out_update_failed' })
    await createSvnSyncOutFlow(deps(log)).handleSyncOutAction('syncOutContinue')
    expect(log).toContain('dialogError:sync_out_update_failed')
    expect(log).toContain('refreshSvnSyncState')
    expect(log.filter((l) => l.startsWith('confirm:'))).toHaveLength(1) // 没有结局提示
  })
})

describe('交付前的 merge driver 确认', () => {
  it('guardMerge 返回 false 时「交付」不预览、不执行', async () => {
    const log: string[] = []
    await createSvnSyncOutFlow({ ...deps(log), guardMerge: async () => false }).handleSvnDeliver()
    expect(store.svnSyncOutPreview).not.toHaveBeenCalled()
    expect(store.svnSyncOut).not.toHaveBeenCalled()
  })

  it('「交付」先过 guardMerge 再预览', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncOutPreview).mockImplementation(async () => {
      log.push('preview')
      return preview
    })
    vi.mocked(store.svnSyncOut).mockResolvedValue({ kind: 'delivered', rev: 2, branch: 'work', localOnlyLeft: 0 })
    await createSvnSyncOutFlow(deps(log)).handleSvnDeliver()
    expect(log.indexOf('guardMerge')).toBeGreaterThanOrEqual(0)
    expect(log.indexOf('guardMerge')).toBeLessThan(log.indexOf('preview'))
  })

  it('横幅上的「继续」要过 guardMerge，返回 false 时不调后端', async () => {
    const log: string[] = []
    await createSvnSyncOutFlow({ ...deps(log), guardMerge: async () => false }).handleSyncOutAction('syncOutContinue')
    expect(store.svnSyncOutContinue).not.toHaveBeenCalled()
  })

  it('「放弃 / 清理」不合并，不过 guardMerge', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncOutAbort).mockResolvedValue({ kind: 'aborted' } as never)
    const flow = createSvnSyncOutFlow({ ...deps(log), guardMerge: async () => false })
    await flow.handleSyncOutAction('syncOutAbort')
    await flow.handleSyncOutAction('syncOutCleanup')
    expect(store.svnSyncOutAbort).toHaveBeenCalledTimes(2)
  })
})

describe('补丁落回冲突时的「我已解决，继续」不再弹确认框（2026-11-19 使用报告问题 2）', () => {
  const at = (phase: string) => () => ({
    locked: false,
    holder: null,
    unfinished: { flow: 'sync_out' as const, phase: phase as never, opid: 'x', startedAt: 1 },
  })
  const busyLog = (log: string[]) => (b: { id: string; label: string } | null) => {
    log.push(b ? `busy:${b.id}|${b.label}` : 'busy:null')
  }

  it('restoring-branch：按钮显示「正在继续…」→ 执行 → 刷新 → 复位，不弹确认框', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncOutContinue).mockImplementation(async () => {
      log.push('continue')
      return { kind: 'delivered', rev: 2, branch: 'work', localOnlyLeft: 0 }
    })

    await createSvnSyncOutFlow({
      ...deps(log),
      syncState: at('restoring-branch'),
      setSyncActionBusy: busyLog(log),
    }).handleSyncOutAction('syncOutContinue')

    expect(log.slice(0, 8)).toEqual([
      'clearError',
      'guardMerge',
      'busy:syncOutContinue|正在继续…',
      'continue',
      'refreshSvnSyncState',
      'refreshSvnSync',
      'fetchSvnLog',
      'busy:null',
    ])
    expect(log).not.toContain('confirm:继续交付')
  })

  it('restoring-branch 还有没解决的文件：刷新横幅、进错误条、按钮复位', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncOutContinue).mockRejectedValue({ code: 'sync_out_unresolved' })

    await createSvnSyncOutFlow({
      ...deps(log),
      syncState: at('restoring-branch'),
      setSyncActionBusy: busyLog(log),
    }).handleSyncOutAction('syncOutContinue')

    expect(log.some((l) => l.startsWith('confirm:'))).toBe(false)
    expect(log.indexOf('refreshSvnSyncState')).toBeGreaterThan(-1)
    expect(log.indexOf('refreshSvnSyncState')).toBeLessThan(log.indexOf('reportError:sync_out_unresolved'))
    expect(log.at(-1)).toBe('busy:null')
  })

  it('restoring-branch 继续之后又停在冲突上：按钮先复位，结局照常报出来', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncOutContinue).mockResolvedValue({ kind: 'rebaseConflict', rev: 7, files: ['Code/P.cs'] })

    await createSvnSyncOutFlow({
      ...deps(log),
      syncState: at('restoring-branch'),
      setSyncActionBusy: busyLog(log),
    }).handleSyncOutAction('syncOutContinue')

    const notice = log.findIndex((l) => l.startsWith('confirm:'))
    expect(notice).toBeGreaterThan(log.indexOf('busy:null'))
    expect(log.filter((l) => l.startsWith('confirm:'))).toHaveLength(1) // 只有结局提示，没有确认框
    expect(log).not.toContain('confirm:继续交付')
  })

  it('post-commit 的「继续交付」照旧确认：框里说要先更新白名单目录、需要网络', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncOutContinue).mockResolvedValue({ kind: 'delivered', rev: 2, branch: 'work', localOnlyLeft: 0 })
    await createSvnSyncOutFlow({
      ...deps(log),
      syncState: at('post-commit'),
      setSyncActionBusy: busyLog(log),
    }).handleSyncOutAction('syncOutContinue')
    expect(log).toContain('confirm:继续交付')
    expect(log.some((l) => l.startsWith('busy:'))).toBe(false)
  })
})
