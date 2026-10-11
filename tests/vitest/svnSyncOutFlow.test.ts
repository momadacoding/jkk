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
    expect(log).toEqual(['clearError', 'reportError:sync_out_behind_baseline'])
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
