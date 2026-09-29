import { beforeEach, describe, expect, it, vi } from 'vitest'

// 只替换发往后端的四个 invoke 包装；流程本身（先后顺序、清错、弹框）是被测对象
vi.mock('@/store/repoStore', () => ({
  svnSyncInPreview: vi.fn(),
  svnSyncIn: vi.fn(),
  svnSyncContinue: vi.fn(),
  svnSyncAbort: vi.fn(),
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
  }
}

beforeEach(() => {
  vi.mocked(store.svnSyncInPreview).mockReset()
  vi.mocked(store.svnSyncIn).mockReset()
  vi.mocked(store.svnSyncContinue).mockReset()
  vi.mocked(store.svnSyncAbort).mockReset()
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

    expect(log).toEqual(['clearError', 'reportError:sync_in_merge_in_range'])
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
