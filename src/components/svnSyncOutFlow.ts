/**
 * 「交付」与横幅上交付那一组按钮的交互流程（spec §8）。
 *
 * 与 `svnSyncFlow.ts` 同一结构：不依赖 React，依赖全部从参数进来，能在 node 环境里测先后顺序。
 * 区别是确认要用户写提交信息，所以对话框是 `DeliverDialog`，不是 `ConfirmDialog`。
 */
import {
  svnSyncOut,
  svnSyncOutAbort,
  svnSyncOutContinue,
  svnSyncOutPreview,
} from '../store/repoStore'
import {
  describeDeliverPreview,
  describeSyncOutAction,
  describeSyncOutOutcome,
  expectFromDeliverPreview,
  syncOutContinueNeedsConfirm,
  type DeliverDialogModel,
  type SyncOutActionId,
  type SyncOutOutcome,
} from '../engine/svnSyncOut'
import type { SyncState, SyncStateUnreadable } from '../engine/svnJournal'
import { opensConflictList } from '../engine/svnConflicts'
import type { ConfirmRequest } from './ConfirmDialog'
import {
  createAfterSync,
  exclusive,
  runOnBanner,
  showOutcomeNotice,
  type SyncActionBusy,
} from './svnFlowShared'

/** 交付对话框的请求。`action` 抛错时对话框留在原地显示错误。 */
export interface DeliverRequest {
  model: DeliverDialogModel
  /** 上一次没交付成功时写下的提交信息（spec §13：保留用户输入的提交信息）。 */
  initialMessage: string
  onMessageChange: (message: string) => void
  action: (message: string) => Promise<void>
}

export interface SvnSyncOutFlowDeps {
  repoPath: string
  /** 是否已有一次流程在走。调用方用 `useRef` 持有，理由同 `svnSyncFlow`。 */
  running: { current: boolean }
  /** 提交信息草稿。调用方用 `useRef` 持有：对话框关了、下次打开还要在。 */
  draft: { current: string }
  /** 取**当前**的同步状态（横幅按钮按它的 phase 选文案）。 */
  syncState: () => SyncState | SyncStateUnreadable | null
  deliver: (request: DeliverRequest) => Promise<boolean>
  confirm: (request: ConfirmRequest) => Promise<boolean>
  /**
   * 合并前的 merge driver 确认（spec §4.6）：更新 / 交付里的 rebase、补丁落回、merge-tree
   * 都会调用 driver。返回 false 时什么都不做。必填：漏接一个入口就是缺陷。
   */
  guardMerge: () => Promise<boolean>
  clearError: () => void
  reportError: (err: unknown) => void
  refreshSvnSyncState: (path: string) => Promise<void>
  refreshSvnSync: (path: string) => Promise<void>
  fetchSvnLog: (path: string) => Promise<unknown>
  /** 未收尾横幅上执行中的那颗按钮，与更新流程共用（同一条横幅）。 */
  setSyncActionBusy: (busy: SyncActionBusy | null) => void
  /**
   * 同 `SvnSyncFlowDeps.offerSvnConflicts`：交付的前置检查、执行或「继续」报「有冲突没解决」（含 D6 的
   * `sync_out_update_conflict`）时弹冲突列表。不等、不抛。
   */
  offerSvnConflicts: () => void
}

export function createSvnSyncOutFlow(d: SvnSyncOutFlowDeps) {
  const afterSync = createAfterSync(d)

  /**
   * 失败也可能留下或推进了记录（D5 状态未知、D6 失败、继续时从 post-commit 走到
   * publishing-baseline 再停下），横幅要立刻跟上——停在旧 phase 的话按钮与文案都是错的。
   * 错误原样抛回给对话框显示。
   *
   * 后端无论成败都补发 `repo-changed`、监听器也会刷同步状态，所以它对最终状态不是必需的；
   * 它保证的是**先后**：用户看到错误时横幅已经是新 phase（第 77 号 issue 的确认，见决策日志）。
   */
  const refreshing = async <T>(run: () => Promise<T>, seen?: { conflicted: boolean }): Promise<T> => {
    try {
      return await run()
    } catch (err) {
      // 错误留在对话框里，流程看不到错误码：「有冲突没解决」记下来，框关掉之后弹冲突列表
      if (seen && opensConflictList(err)) seen.conflicted = true
      await afterSync()
      throw err
    }
  }

  const showOutcome = (o: SyncOutOutcome | null) =>
    showOutcomeNotice(d.confirm, o && describeSyncOutOutcome(o))

  /** 「交付」：预览 → 对话框（写信息、确认）→ 执行。 */
  const svnDeliver = async () => {
    // 同 svnSyncFlow：上一次被前置检查拒绝留下的错误条说的是当时的仓库
    d.clearError()
    // 交付里的 rebase、补丁落回、慢路径的 merge-tree 都会调用 driver；先于预览，理由同 svnSyncFlow
    if (!(await d.guardMerge())) return
    let preview
    try {
      preview = await svnSyncOutPreview(d.repoPath)
    } catch (err) {
      d.reportError(err)
      if (opensConflictList(err)) d.offerSvnConflicts()
      return
    }
    // 用盒子而不是 `let outcome`：闭包里的赋值 TS 的控制流分析看不见
    const box: { outcome: SyncOutOutcome | null; conflicted: boolean } = {
      outcome: null,
      conflicted: false,
    }
    await d.deliver({
      model: describeDeliverPreview(preview),
      initialMessage: d.draft.current,
      onMessageChange: (m) => {
        d.draft.current = m
      },
      action: async (message) => {
        box.outcome = await refreshing(
          () => svnSyncOut(d.repoPath, expectFromDeliverPreview(preview, message)),
          box,
        )
        // 只在成功后清：失败时草稿留给下一次（spec §13）
        d.draft.current = ''
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
    if (box.conflicted) d.offerSvnConflicts()
  }

  /**
   * 横幅上交付那一组按钮。「清理」与「停止」都走放弃命令（计划 B8）。补丁落回冲突时的「继续」
   * 不弹确认框（`syncOutContinueNeedsConfirm`，2026-11-19 使用报告问题 2）。
   */
  const syncOutAction = async (id: SyncOutActionId) => {
    d.clearError()
    // 只有「继续」会接着合并；「清理」与「停止」都走放弃命令
    if (id === 'syncOutContinue' && !(await d.guardMerge())) return
    const s = d.syncState()
    const phase = s && !('readError' in s) && s.unfinished ? s.unfinished.phase : 'prepare'
    const model = describeSyncOutAction(id, phase)
    if (id === 'syncOutContinue' && !syncOutContinueNeedsConfirm(phase)) {
      // 不套 `refreshing`：`runOnBanner` 失败时自己先刷新再报。错误码在这里先记下
      const seen = { conflicted: false }
      const outcome = await runOnBanner(d, afterSync, { id, label: model.busyLabel }, () =>
        svnSyncOutContinue(d.repoPath).catch((err: unknown) => {
          if (opensConflictList(err)) seen.conflicted = true
          throw err
        }),
      )
      await showOutcome(outcome)
      if (seen.conflicted) d.offerSvnConflicts()
      return
    }
    const box: { outcome: SyncOutOutcome | null; conflicted: boolean } = {
      outcome: null,
      conflicted: false,
    }
    await d.confirm({
      ...model,
      action: async () => {
        box.outcome = await refreshing(
          () => (id === 'syncOutContinue' ? svnSyncOutContinue(d.repoPath) : svnSyncOutAbort(d.repoPath)),
          box,
        )
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
    if (box.conflicted) d.offerSvnConflicts()
  }

  return {
    handleSvnDeliver: () => exclusive(d.running, svnDeliver),
    handleSyncOutAction: (id: SyncOutActionId) => exclusive(d.running, () => syncOutAction(id)),
  }
}
