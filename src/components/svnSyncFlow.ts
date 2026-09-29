/**
 * 「更新」与横幅上「继续 / 放弃」的交互流程（spec §7、§7.6）。
 *
 * 从 `RepoTab` 里拆出来，是为了能在 node 环境里直接测先后顺序（清错 → 预览 → 确认 →
 * 执行 → 刷新 → 结局），不必渲染整个标签页。不依赖 React：依赖全部从参数进来。
 */
import { svnSyncAbort, svnSyncContinue, svnSyncIn, svnSyncInPreview } from '../store/repoStore'
import {
  describeSyncAction,
  describeSyncInPreview,
  describeSyncOutcome,
  expectFromPreview,
  type SyncActionId,
  type SyncInOutcome,
} from '../engine/svnSyncIn'
import type { SyncState, SyncStateUnreadable } from '../engine/svnJournal'
import type { ConfirmRequest } from './ConfirmDialog'

export interface SvnSyncFlowDeps {
  repoPath: string
  /** 取**当前**的同步状态（横幅上的继续 / 放弃按它的 phase 选文案）。 */
  syncState: () => SyncState | SyncStateUnreadable | null
  confirm: (request: ConfirmRequest) => Promise<boolean>
  clearError: () => void
  reportError: (err: unknown) => void
  refreshSvnSyncState: (path: string) => Promise<void>
  refreshSvnSync: (path: string) => Promise<void>
  fetchSvnLog: (path: string) => Promise<unknown>
}

export function createSvnSyncFlow(d: SvnSyncFlowDeps) {
  /** 同步收尾之后的刷新。`repo-changed` 由后端补发，这里只补它不管的 SVN 侧状态。 */
  const afterSync = async () => {
    await d.refreshSvnSyncState(d.repoPath)
    await d.refreshSvnSync(d.repoPath)
    void d.fetchSvnLog(d.repoPath)
  }

  /**
   * 冲突、撞名、放弃这几种结局要让用户看见；成功不打扰。
   *
   * 走 `confirm` 只是借它的对话框外观——取消与确认在这里等价。
   */
  const showOutcome = async (o: SyncInOutcome | null) => {
    const n = o && describeSyncOutcome(o)
    if (!n) return
    await d.confirm({ title: n.title, message: n.detail, confirmLabel: '知道了' })
  }

  /**
   * 「更新」（spec §7）：预览 → 确认 → 执行。
   *
   * 预览失败（前置检查不过）直接报错，一个字节都没动。执行走 `action`：
   * 它要跑完整的 svn update（网络），确认完就关框的话界面上几秒毫无动静。
   * 执行出错时对话框留在原地显示错误——后端的错误文案里已经写了回退的结果。
   */
  const handleSvnUpdate = async () => {
    // 顶部错误条是常驻的（点 × 才关）。上一次「更新」被前置检查拒绝留下的那条说的是
    // 当时的仓库，用户改好之后再点更新时它已经过时，还会和新横幅叠在一起（2026-09-30
    // E2E 场景 6 → 7）。重试就是新的一次：先清旧的，这次再失败自然会报新的
    d.clearError()
    let preview
    try {
      preview = await svnSyncInPreview(d.repoPath)
    } catch (err) {
      d.reportError(err)
      return
    }
    // 用盒子而不是 `let outcome`：闭包里的赋值 TS 的控制流分析看不见，
    // `if (outcome)` 会被收窄成 never
    const box: { outcome: SyncInOutcome | null } = { outcome: null }
    await d.confirm({
      ...describeSyncInPreview(preview),
      action: async () => {
        box.outcome = await svnSyncIn(d.repoPath, expectFromPreview(preview))
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
  }

  /** 横幅上的继续 / 放弃（spec §7.6）。 */
  const handleSyncAction = async (id: SyncActionId) => {
    d.clearError() // 同 handleSvnUpdate
    const s = d.syncState()
    const phase = s && !('readError' in s) && s.unfinished ? s.unfinished.phase : 'start'
    const box: { outcome: SyncInOutcome | null } = { outcome: null }
    await d.confirm({
      ...describeSyncAction(id, phase),
      action: async () => {
        box.outcome =
          id === 'syncContinue' ? await svnSyncContinue(d.repoPath) : await svnSyncAbort(d.repoPath)
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
  }

  return { handleSvnUpdate, handleSyncAction }
}
