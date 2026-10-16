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
  type DeliverDialogModel,
  type SyncOutActionId,
  type SyncOutOutcome,
} from '../engine/svnSyncOut'
import type { SyncState, SyncStateUnreadable } from '../engine/svnJournal'
import type { ConfirmRequest } from './ConfirmDialog'

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
  clearError: () => void
  reportError: (err: unknown) => void
  refreshSvnSyncState: (path: string) => Promise<void>
  refreshSvnSync: (path: string) => Promise<void>
  fetchSvnLog: (path: string) => Promise<unknown>
}

export function createSvnSyncOutFlow(d: SvnSyncOutFlowDeps) {
  /**
   * 收尾之后的刷新。`repo-changed` 由后端补发，这里只补它不管的 SVN 侧状态。
   *
   * **自己不抛**：刷新失败进错误条，不能改写交付本身的结局（最终 review 第 3 条）——
   * 成功后抛的话对话框留在原地报错、「交付」还能点，用户以为没提交上去；失败后抛的话
   * 后端原文（D6 失败时那是用户唯一知道「提交已经成功、点继续」的地方）被顶掉。
   */
  const afterSync = async () => {
    try {
      await d.refreshSvnSyncState(d.repoPath)
      await d.refreshSvnSync(d.repoPath)
    } catch (err) {
      d.reportError(err)
    }
    void d.fetchSvnLog(d.repoPath)
  }

  /**
   * 失败也可能留下或推进了记录（D5 状态未知、D6 失败、继续时从 post-commit 走到
   * publishing-baseline 再停下），横幅要立刻跟上——停在旧 phase 的话按钮与文案都是错的。
   * 错误原样抛回给对话框显示。
   */
  const refreshing = async <T>(run: () => Promise<T>): Promise<T> => {
    try {
      return await run()
    } catch (err) {
      await afterSync()
      throw err
    }
  }

  /** 走 `confirm` 只是借它的对话框外观——取消与确认在这里等价。 */
  const showOutcome = async (o: SyncOutOutcome | null) => {
    const n = o && describeSyncOutOutcome(o)
    if (!n) return
    await d.confirm({ title: n.title, message: n.detail, confirmLabel: '知道了' })
  }

  /** 同一时间只走一个流程（理由同 `svnSyncFlow` 的 `exclusive`）。 */
  const exclusive = async (run: () => Promise<void>) => {
    if (d.running.current) return
    d.running.current = true
    try {
      await run()
    } finally {
      d.running.current = false
    }
  }

  /** 「交付」：预览 → 对话框（写信息、确认）→ 执行。 */
  const svnDeliver = async () => {
    // 同 svnSyncFlow：上一次被前置检查拒绝留下的错误条说的是当时的仓库
    d.clearError()
    let preview
    try {
      preview = await svnSyncOutPreview(d.repoPath)
    } catch (err) {
      d.reportError(err)
      return
    }
    // 用盒子而不是 `let outcome`：闭包里的赋值 TS 的控制流分析看不见
    const box: { outcome: SyncOutOutcome | null } = { outcome: null }
    await d.deliver({
      model: describeDeliverPreview(preview),
      initialMessage: d.draft.current,
      onMessageChange: (m) => {
        d.draft.current = m
      },
      action: async (message) => {
        box.outcome = await refreshing(() =>
          svnSyncOut(d.repoPath, expectFromDeliverPreview(preview, message)),
        )
        // 只在成功后清：失败时草稿留给下一次（spec §13）
        d.draft.current = ''
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
  }

  /** 横幅上交付那一组按钮。「清理」与「停止」都走放弃命令（计划 B8）。 */
  const syncOutAction = async (id: SyncOutActionId) => {
    d.clearError()
    const s = d.syncState()
    const phase = s && !('readError' in s) && s.unfinished ? s.unfinished.phase : 'prepare'
    const box: { outcome: SyncOutOutcome | null } = { outcome: null }
    await d.confirm({
      ...describeSyncOutAction(id, phase),
      action: async () => {
        box.outcome = await refreshing(() =>
          id === 'syncOutContinue' ? svnSyncOutContinue(d.repoPath) : svnSyncOutAbort(d.repoPath),
        )
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
  }

  return {
    handleSvnDeliver: () => exclusive(svnDeliver),
    handleSyncOutAction: (id: SyncOutActionId) => exclusive(() => syncOutAction(id)),
  }
}
