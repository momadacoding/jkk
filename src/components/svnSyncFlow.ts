/**
 * 「更新」与横幅上「继续 / 放弃」的交互流程（spec §7、§7.6）。
 *
 * 从 `RepoTab` 里拆出来，是为了能在 node 环境里直接测先后顺序（清错 → 预览 → 确认 →
 * 执行 → 刷新 → 结局），不必渲染整个标签页。不依赖 React：依赖全部从参数进来。
 */
import {
  svnFollow,
  svnFollowAuto,
  svnFollowContinue,
  svnFollowManual,
  svnSyncAbort,
  svnSyncContinue,
  svnSyncIn,
  svnSyncInPreview,
} from '../store/repoStore'
import {
  AUTO_FOLLOW_OFF,
  describeCheckoutBehind,
  describeFollowConflict,
  describeFollowContinue,
  describeFollowOutcome,
  describeManualFollow,
  describeManualFollowAction,
  describeManualFollowOutcome,
  type AutoFollowOutcome,
  type FollowActionId,
  type FollowOutcome,
  type FollowReport,
} from '../engine/svnFollow'
import {
  describeSyncAction,
  describeSyncInPreview,
  describeSyncOutcome,
  expectFromPreview,
  type SyncActionId,
  type SyncConfirmModel,
  type SyncInOutcome,
} from '../engine/svnSyncIn'
import type { SyncState, SyncStateUnreadable } from '../engine/svnJournal'
import type { ConfirmRequest } from './ConfirmDialog'

export interface SvnSyncFlowDeps {
  repoPath: string
  /**
   * 是否已有一次流程在走。调用方用 `useRef` 持有：`RepoTab` 每次渲染都重新 create，
   * 放在闭包里的标记会跟着重置。
   */
  running: { current: boolean }
  /** 取**当前**的同步状态（横幅上的继续 / 放弃按它的 phase 选文案）。 */
  syncState: () => SyncState | SyncStateUnreadable | null
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
  /** 跟进执行中。横幅上的按钮据此禁用：跟进要走服务器体检，这几秒里没有框挡着。 */
  setFollowing: (busy: boolean) => void
  /** 跟进之后刷新 HEAD、提交图、本地改动与落后基线的分支。可以抛，流程接住。 */
  refreshAfterFollow: () => Promise<void>
  /** 弹跟进报告，关掉时结束。 */
  showFollowReport: (report: FollowReport) => Promise<void>
  /** 切分支并刷新。失败要抛：「切换并手动跟进」据此停下。 */
  checkoutBranch: (name: string) => Promise<void>
  /**
   * 自动跟进用的 merge driver 检查（spec §4.6）：有风险时返回给横幅的原因，没有风险返回 `null`。
   * 不弹框：自动跟进不能停下来问人，有风险就不自动，留给会弹确认框的「跟进」按钮。
   */
  mergeRiskForAuto: () => Promise<string | null>
  /** 没自动跟进的原因，给横幅；`null` 清掉。`key` 是 `autoFollowKey` 算的现场。 */
  setAutoFollowSkip: (skip: { key: string; message: string } | null) => void
  /** 自动跟进完成：挂那条不打断操作的提示，详情是同一份报告。 */
  showAutoFollowed: (report: FollowReport) => void
}

export function createSvnSyncFlow(d: SvnSyncFlowDeps) {
  /**
   * 同步收尾之后的刷新。`repo-changed` 由后端补发，这里只补它不管的 SVN 侧状态。
   *
   * **自己不抛**，理由同 `svnSyncOutFlow` 的 `afterSync`：它跑在对话框的 `action` 里，
   * 更新已经落地之后再抛，对话框会留在原地报错、「更新」还能点，用户以为没更新上。
   * 刷新失败只进错误条。
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
   * 跟进之后多刷一份。`repo-changed` 也会补发，这里不靠它：报告弹出来时界面要已经是新的。
   * 同样不抛，跟进已经落地了。
   */
  const afterFollow = async () => {
    try {
      await d.refreshAfterFollow()
    } catch (err) {
      d.reportError(err)
    }
    await afterSync()
  }

  /** 冲突、撞名、放弃这几种结局要让用户看见；成功不打扰。`notice` 让它只有一个按钮。 */
  const showOutcome = async (o: SyncInOutcome | null, manual = false) => {
    const n = o && (manual ? describeManualFollowOutcome(o) : describeSyncOutcome(o))
    if (!n) return
    await d.confirm({ title: n.title, message: n.detail, confirmLabel: '知道了', notice: n.tone })
  }

  /**
   * 同一时间只走一个流程。预览要跑 `svn status -u`（网络），那几秒里界面上没有框挡着，
   * 工具栏的「更新」和横幅上的按钮都点得到；两个流程的确认框会互相顶替
   * （PR #49 Codex review #5）。后端的锁拦得住第二次执行，拦不住界面的错乱。
   */
  const exclusive = async (run: () => Promise<void>) => {
    if (d.running.current) return
    d.running.current = true
    try {
      await run()
    } finally {
      d.running.current = false
    }
  }

  /**
   * 「更新」（spec §7）：预览 → 确认 → 执行。
   *
   * 预览失败（前置检查不过）直接报错，一个字节都没动。执行走 `action`：
   * 它要跑完整的 svn update（网络），确认完就关框的话界面上几秒毫无动静。
   * 执行出错时对话框留在原地显示错误——后端的错误文案里已经写了回退的结果。
   */
  const svnUpdate = async () => {
    // 顶部错误条是常驻的（点 × 才关）。上一次「更新」被前置检查拒绝留下的那条说的是
    // 当时的仓库，用户改好之后再点更新时它已经过时，还会和新横幅叠在一起（2026-09-30
    // E2E 场景 6 → 7）。重试就是新的一次：先清旧的，这次再失败自然会报新的
    d.clearError()
    // 先于预览：预览要走网络，用户在这里取消的话那几秒白等
    if (!(await d.guardMerge())) return
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
  const syncAction = async (id: SyncActionId) => {
    d.clearError() // 同 svnUpdate
    // 「继续」会接着跑 rebase；「放弃」只是回退，不合并
    if (id === 'syncContinue' && !(await d.guardMerge())) return
    const s = d.syncState()
    const unfinished = s && !('readError' in s) ? s.unfinished : null
    const phase = unfinished ? unfinished.phase : 'start'
    // 手动跟进的记录走同一对命令，只换说法（后端 `take_over` 接手两种 flow）
    const manual = unfinished?.flow === 'follow_manual'
    const box: { outcome: SyncInOutcome | null } = { outcome: null }
    await d.confirm({
      ...(manual ? describeManualFollowAction(id, phase) : describeSyncAction(id, phase)),
      action: async () => {
        box.outcome =
          id === 'syncContinue' ? await svnSyncContinue(d.repoPath) : await svnSyncAbort(d.repoPath)
        await afterSync()
      },
    })
    await showOutcome(box.outcome, manual)
  }

  /**
   * 手动跟进（分析 §6.5）：跟进冲突时的兜底，会写磁盘，所以永远走确认框，绝不自动发起。
   * 结局与冲突出口沿用更新，后续的继续 / 放弃走 {@link syncAction}。调用方负责先过 merge driver。
   */
  const confirmManualFollow = async (model: SyncConfirmModel) => {
    const box: { outcome: SyncInOutcome | null } = { outcome: null }
    await d.confirm({
      ...model,
      action: async () => {
        box.outcome = await svnFollowManual(d.repoPath)
        await afterSync()
      },
    })
    await showOutcome(box.outcome, true)
  }

  /** 「切换并手动跟进」切过去之后的那一步。调用方已在 `exclusive` 里。 */
  const manualFollow = async () => {
    d.clearError()
    // rebase 与放回都会调用 merge driver
    if (!(await d.guardMerge())) return
    await confirmManualFollow(describeManualFollow())
  }

  /** 按跟进的结局走下一步。`needsManual` 这时 merge driver 已经确认过了。 */
  const settleFollow = async (o: FollowOutcome) => {
    if (o.kind === 'done') {
      await d.showFollowReport(o.report)
      return
    }
    if (o.kind === 'needsManual') {
      await confirmManualFollow(describeFollowConflict(o))
      return
    }
    const n = describeFollowOutcome(o)
    await d.confirm({ title: n.title, message: n.detail, confirmLabel: '知道了', notice: n.tone })
  }

  /**
   * 横幅上的「跟进」（分析 §6.5）。不写工作区，所以不要确认框；重放走 merge-tree，要过
   * merge driver 确认。跑的时候没有框挡着，靠 `setFollowing` 让按钮有反应。
   */
  const follow = async () => {
    d.clearError()
    if (!(await d.guardMerge())) return
    d.setFollowing(true)
    let outcome: FollowOutcome
    try {
      outcome = await svnFollow(d.repoPath)
      await afterFollow()
    } catch (err) {
      d.reportError(err)
      return
    } finally {
      d.setFollowing(false)
    }
    await settleFollow(outcome)
  }

  /**
   * 自动跟进（事故分析 6.12）：判定写回 ExternallyAdvanced 之后由 `RepoTab` 发起，与横幅上的「跟进」
   * 共用 `exclusive`，在跑时谁都不重入。
   *
   * 和按钮的差别：不清错误条（用户没做任何事）、不弹框（merge driver 有风险就不自动）、不自动时
   * 只在横幅上写一行原因、完成时挂一条不打断操作的提示。出错（动过 ref 之后才失败）照常进错误条，
   * 文案里写着此刻的状态。
   */
  const autoFollow = async (key: string) => {
    const risk = await d.mergeRiskForAuto()
    if (risk) {
      d.setAutoFollowSkip({ key, message: risk })
      return
    }
    d.setFollowing(true)
    let outcome: AutoFollowOutcome
    try {
      outcome = await svnFollowAuto(d.repoPath)
      if (outcome.kind === 'done') await afterFollow()
    } catch (err) {
      d.reportError(err)
      return
    } finally {
      d.setFollowing(false)
    }
    if (outcome.kind === 'skipped') {
      d.setAutoFollowSkip(outcome.code === AUTO_FOLLOW_OFF ? null : { key, message: outcome.message })
      return
    }
    d.setAutoFollowSkip(null)
    d.showAutoFollowed(outcome.report)
  }

  /**
   * 切到落后基线的分支（分析 §6.11.3）。切分支被工作区改动挡住时报错就停，不替用户 stash：
   * 那是另一次要确认的操作，手动跟进自己的 stash 要等切过去之后才轮得到。
   */
  const checkoutBehind = async (branch: string) => {
    d.clearError()
    const { alternativeLabel, ...model } = describeCheckoutBehind(branch)
    const box = { onlySwitch: false }
    const follow = await d.confirm({
      ...model,
      alternative: {
        label: alternativeLabel,
        onSelect: () => {
          box.onlySwitch = true
        },
      },
    })
    if (!follow && !box.onlySwitch) return
    try {
      await d.checkoutBranch(branch)
    } catch (err) {
      d.reportError(err)
      return
    }
    if (follow) await manualFollow()
  }

  /**
   * 横幅上跟进记录的「继续」（计划 Task 10）。恢复走不下去时会重新跟进，重放要走 merge-tree，
   * 所以和更新的「继续」一样先过 merge driver 确认。
   */
  const followContinue = async () => {
    d.clearError()
    if (!(await d.guardMerge())) return
    const box: { outcome: FollowOutcome | null } = { outcome: null }
    await d.confirm({
      ...describeFollowContinue(),
      action: async () => {
        box.outcome = await svnFollowContinue(d.repoPath)
        await afterFollow()
      },
    })
    if (box.outcome) await settleFollow(box.outcome)
  }

  return {
    handleSvnUpdate: () => exclusive(svnUpdate),
    handleFollow: () => exclusive(follow),
    /**
     * 有流程在走时什么都不做、返回 `false`：调用方不记这个现场，等那个流程收尾刷新判定之后再来。
     * 判断是同步的（`running` 在第一个 await 之前就置上），调用方在 `true` 时可以放心记下。
     */
    handleAutoFollow: (key: string): boolean => {
      if (d.running.current) return false
      void exclusive(() => autoFollow(key))
      return true
    },
    // 没有单独的「手动跟进」入口（Review 3 第 4 条）：它只经由跟进冲突的确认框与「切换并手动跟进」
    // 发起，两处都已在 `exclusive` 里，见 `settleFollow` 与 `checkoutBehind`
    handleCheckoutBehind: (branch: string) => exclusive(() => checkoutBehind(branch)),
    handleSyncAction: (id: SyncActionId | FollowActionId) =>
      exclusive(() => (id === 'followContinue' ? followContinue() : syncAction(id))),
  }
}
