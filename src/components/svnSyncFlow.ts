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
  RECHECK_SKIPS,
  describeCheckoutBehind,
  describeFollowConflict,
  describeFollowContinue,
  describeFollowOutcome,
  describeManualFollow,
  describeManualFollowAction,
  describeManualFollowOutcome,
  heldBackScene,
  pinHeldBack,
  type AutoFollowOutcome,
  type HeldBackPin,
  type AutoFollowSkip,
  type FollowActionId,
  type FollowOutcome,
  type FollowReport,
  type SyncAfterFollow,
} from '../engine/svnFollow'
import {
  continueNeedsConfirm,
  describeSyncAction,
  describeSyncInPreview,
  describeSyncOutcome,
  expectFromPreview,
  type SyncActionId,
  type SyncConfirmModel,
  type SyncInOutcome,
} from '../engine/svnSyncIn'
import type { SyncState, SyncStateUnreadable } from '../engine/svnJournal'
import { perfMark, perfSpan } from '../store/perf'
import type { ConfirmRequest } from './ConfirmDialog'
import {
  createAfterSync,
  exclusive as sharedExclusive,
  runOnBanner,
  showOutcomeNotice,
  type SyncActionBusy,
} from './svnFlowShared'

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
  /**
   * 同步之后的强制复查。流程在命令返回的同一拍里调它，不先等别的刷新：store 靠这一点与后端补发的
   * `repo-changed` 共用一轮（`repoStore` 的 `refreshSvnSync`）。
   */
  refreshSvnSync: (path: string) => Promise<void>
  /**
   * 跟进命令发起前取一次号（`repoStore` 的 `svnSyncMark`），写回带回的判定时交回去：期间有检测写回过就不用
   * 带回的那份。
   */
  svnSyncMark: () => number
  /**
   * 跟进带回的判定直接写回（`repoStore` 的 `applyFollowedSync`）。返回 `true` 时跟进之后的刷新不再调
   * `refreshSvnSync`（2026-11-24 分析方案 4）；`false`（仓库已切走、期间有检测写回过、戳没记过、别的 HEAD
   * 在排队，见 `canApplyFollowedSync`）照原来复查。
   */
  applyFollowedSync: (path: string, sync: SyncAfterFollow, since: number) => boolean
  fetchSvnLog: (path: string) => Promise<unknown>
  /**
   * 跟进执行中，含跟进之后的刷新。横幅上的按钮据此禁用：跟进要走服务器体检，这几秒里没有框挡着；
   * 刷新写回之前横幅说的还是跟进前的事，那时点「跟进」只会白跑一次。
   */
  setFollowing: (busy: boolean) => void
  /**
   * 未收尾横幅上执行中的那颗按钮。rebase 冲突时的「继续」不弹确认框，进度只能显示在按钮上；
   * 与交付流程共用同一份状态（同一条横幅）。
   */
  setSyncActionBusy: (busy: SyncActionBusy | null) => void
  /** 跟进之后刷新 HEAD、提交图、本地改动与落后基线的分支。可以抛，流程接住。 */
  refreshAfterFollow: () => Promise<void>
  /**
   * 跟进之后重读 TortoiseSVN 钩子状态。不等、不抛（读失败自己记 console）。流程在跟进后的刷新**全部落定之后**
   * 才调：它要跑一遍全量 `svn status`，与刷新里的判定并行时两遍抢磁盘，各自都慢（2026-11-24 分析方案 3）。
   */
  refreshHooksHealth: () => void
  /**
   * 弹跟进报告，关掉时结束。报告里带着这次命令的用时（`FollowReport.elapsedMs`）。
   *
   * `refreshed` 给了，说明报告是抢在跟进之后的刷新前面弹的：报告里会写工作区的动作（放回 Local-Only
   * 补丁）要等它落定再做。它不会 reject。
   */
  showFollowReport: (report: FollowReport, refreshed?: Promise<void>) => Promise<void>
  /** 切分支并刷新。失败要抛：「切换并手动跟进」据此停下。 */
  checkoutBranch: (name: string) => Promise<void>
  /**
   * 自动跟进用的 merge driver 检查（spec §4.6）：有风险时返回给横幅的原因，没有风险返回 `null`。
   * 不弹框：自动跟进不能停下来问人，有风险就不自动，留给会弹确认框的「跟进」按钮。
   */
  mergeRiskForAuto: () => Promise<string | null>
  /** 设置里自动跟进开着没有。读不出来按开着：后端还会再判一次。 */
  autoFollowEnabled: () => Promise<boolean>
  /**
   * 自动跟进试过的现场（`autoFollowKey`）。`handleAutoFollow` 返回 `true` 时由调用方记下；这里只在两处
   * 改它：部分跟进做完后把剩下的现场也记成试过，被 `RECHECK_SKIPS` 拦下时把这次的现场放掉。
   * 用 ref 的理由同 `running`。
   */
  autoTried: { current: string | null }
  /**
   * 跟进并刷新完之后**此刻**的判定与 HEAD。要现读：流程在跑的这几秒里，闭包里的 state 还是发起时的。
   * 可以抛，流程接住。
   */
  sceneAfterFollow: () => Promise<{
    sync: { verdict: { kind: string } } | null
    head: string | null
  }>
  /**
   * 让调用方再看一次要不要自动跟进。自动跟进**做完**、`running` 放开之后调用（独立 review 中 1）：
   * 跟进后的刷新会写回新的判定，调用方那一次检查落在流程还在走的时候，被 `running` 挡掉、不记现场，
   * 流程收尾之后没有别的东西再触发它。新现场已经记成试过（只剩留下的文件）时这一次是空转；
   * 没记（跟进期间又来了新内容、读不出现场）就照常自动试。被拦下时不调：现场没变，调了就是原地打转。
   */
  nudgeAutoFollow: () => void
  /** 暂时性原因重试前的等待。测试里换成立即返回。 */
  wait: (ms: number) => Promise<void>
  /** 没自动跟进的原因，给横幅；`null` 清掉。`key` 是 `autoFollowKey` 算的现场。 */
  setAutoFollowSkip: (skip: (AutoFollowSkip & { key: string }) | null) => void
  /** 自动跟进完成：挂那条不打断操作的提示，详情是同一份报告。 */
  showAutoFollowed: (report: FollowReport) => void
  /**
   * 收起上一次「已自动跟进」的提示。新的一次跟进（自动、横幅按钮、手动、更新）真的开始时调用：
   * 旧提示说的是上一次的事，留着会和新横幅打架（E2E 2026-11-13：旧提示写「放回了 1 个文件」，
   * 下面的新横幅写「1 个文件缺 Local-Only 补丁」）。只收安静的那种，要你留意的报告留着
   * （`keepOnNewFollow`），所以这里清得早一点（例如更新的前置检查没过）也不丢东西。
   */
  clearAutoFollowed: () => void
  /**
   * 自动跟进完成时记下这一次留下的文件，横幅一直按原因列着（`describeHeldBack`）；没留下就是
   * `null`，撤掉上一次的。这一次没跟进（被拦下、开关关着）时不调：上一次留下的照旧成立。
   */
  setHeldBack: (pin: HeldBackPin | null) => void
}

/**
 * 自动跟进没跑成、但过一会儿多半就能跑的原因：外部 git 拿着索引锁（IDE 看到文件变了跑 `git status`）、
 * 准备期间仓库一直在变、另一次同步正在进行。钩子事件让检测紧跟在 TortoiseSVN 之后，正撞上这些。
 */
const TRANSIENT_SKIPS = new Set(['svn_index_busy', 'svn_follow_state_changed', 'sync_in_progress'])
const TRANSIENT_RETRY_MS = 2000

export function createSvnSyncFlow(d: SvnSyncFlowDeps) {
  const afterSync = createAfterSync(d)

  /**
   * 跟进之后多刷一份：HEAD、提交图、本地改动，加上 SVN 侧的状态。`repo-changed` 也会补发，这里不靠它。
   * 同样不抛，跟进已经落地了。
   *
   * 几项同时发（2026-11-11 使用报告 10.5）：原来一项等一项，公司机器上报告要等它们加起来那么久。
   * 它们写的是 state 里互不相干的字段，后端补发的 `repo-changed` 本来也是同时发的。
   *
   * 命令带回了发布之后的判定（`sync`）就直接写回，不再复查：省掉的是一遍全量 `svn status` 加判定，刷新里
   * 最长的那一项（2026-11-24 分析方案 4）。写回与原来的复查一样在命令返回的这一拍里做，赶在补发的
   * `repo-changed` 读到新 HEAD 之前把它记下。`since` 是命令发起前取的号（`svnSyncMark`）。
   */
  const afterFollow = (sync?: SyncAfterFollow, since?: number) =>
    perfSpan('follow.refresh', async () => {
      const checked =
        sync && since !== undefined ? d.applyFollowedSync(d.repoPath, sync, since) : false
      const [git] = await Promise.allSettled([d.refreshAfterFollow(), afterSync(checked)])
      if (git.status === 'rejected') d.reportError(git.reason)
      // 不 await：不拖住「跟进中」和完成提示，只是错开磁盘
      d.refreshHooksHealth()
    })

  /** 计一次跟进命令的用时，补进报告（`FollowReport.elapsedMs`）。只量命令本身，不含之后的刷新。 */
  const timed = async <T>(run: () => Promise<T>): Promise<{ value: T; ms: number }> => {
    const t0 = performance.now()
    const value = await run()
    return { value, ms: performance.now() - t0 }
  }

  /** 冲突、撞名、放弃这几种结局要让用户看见；成功不打扰。 */
  const showOutcome = (o: SyncInOutcome | null, manual = false) =>
    showOutcomeNotice(
      d.confirm,
      o && (manual ? describeManualFollowOutcome(o) : describeSyncOutcome(o)),
    )

  const exclusive = (run: () => Promise<void>) => sharedExclusive(d.running, run)

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
        // 确认了才清；前置检查在后端没过时也已经收起了，只收安静的提示，不丢报告
        d.clearAutoFollowed()
        box.outcome = await svnSyncIn(d.repoPath, expectFromPreview(preview))
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
  }

  /**
   * 横幅上的继续 / 放弃（spec §7.6）。rebase 冲突时的「继续」不弹确认框，进度显示在按钮上
   * （`continueNeedsConfirm`，2026-11-19 使用报告问题 2）。
   */
  const syncAction = async (id: SyncActionId) => {
    d.clearError() // 同 svnUpdate
    // 「继续」会接着跑 rebase；「放弃」只是回退，不合并
    if (id === 'syncContinue' && !(await d.guardMerge())) return
    const s = d.syncState()
    const unfinished = s && !('readError' in s) ? s.unfinished : null
    const phase = unfinished ? unfinished.phase : 'start'
    // 手动跟进的记录走同一对命令，只换说法（后端 `take_over` 接手两种 flow）
    const manual = unfinished?.flow === 'follow_manual'
    const model = manual ? describeManualFollowAction(id, phase) : describeSyncAction(id, phase)
    if (id === 'syncContinue' && !continueNeedsConfirm(phase)) {
      const outcome = await runOnBanner(d, afterSync, { id, label: model.busyLabel }, () =>
        svnSyncContinue(d.repoPath),
      )
      await showOutcome(outcome, manual)
      return
    }
    const box: { outcome: SyncInOutcome | null } = { outcome: null }
    await d.confirm({
      ...model,
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
        d.clearAutoFollowed()
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
   *
   * # 完成时报告先弹，不等刷新（2026-11-11 使用报告 10.5）
   *
   * 原来等刷新完才弹，为的是报告弹出来时界面已经是新的。但报告是模态框，挡着后面的界面；那几项刷新
   * 加一遍 `svn status` 在公司机器上是好几秒，用户盯着的是「跟进中」的按钮。现在命令一返回就弹，
   * 刷新在框后面跑。
   *
   * 流程要等**报告关掉和刷新完成两件事都结束**才算走完：`running` 提前放开的话，刷新写回 HEAD 的那一刻
   * 判定还是跟进前的 ExternallyAdvanced，自动跟进会按这个过时的现场再发起一次（`RepoTab` 的 effect）。
   * 按钮在刷新完成时复位，不等报告。
   *
   * 拒绝、冲突这些结局不改：照旧刷新完再弹。冲突的下一步是手动跟进（会写磁盘），它的确认框弹出时
   * 不该还有一轮检测在后台读工作副本。
   */
  const follow = async () => {
    d.clearError()
    if (!(await d.guardMerge())) return
    d.clearAutoFollowed()
    d.setFollowing(true)
    // 用盒子的理由同 `svnUpdate`
    const box: { outcome: FollowOutcome | null; reportClosed: Promise<void> | null } = {
      outcome: null,
      reportClosed: null,
    }
    try {
      await perfSpan('follow', async () => {
        const since = d.svnSyncMark()
        const run = await timed(() => svnFollow(d.repoPath))
        box.outcome = run.value
        // 先发刷新再弹报告，中间不 await：复查要赶在命令返回的这一拍里进去
        const refreshed = afterFollow(run.value.sync, since)
        if (run.value.kind === 'done') {
          perfMark('follow.report')
          box.reportClosed = d.showFollowReport({ ...run.value.report, elapsedMs: run.ms }, refreshed)
        }
        await refreshed
      })
    } catch (err) {
      d.reportError(err)
      return
    } finally {
      d.setFollowing(false)
    }
    if (box.reportClosed) await box.reportClosed
    else if (box.outcome) await settleFollow(box.outcome)
  }

  /**
   * 自动跟进（事故分析 6.12）：判定写回 ExternallyAdvanced 之后由 `RepoTab` 发起，与横幅上的「跟进」
   * 共用 `exclusive`，在跑时谁都不重入。
   *
   * 和按钮的差别：不清错误条（用户没做任何事）、不弹框（merge driver 有风险就不自动）、不自动时
   * 只在横幅上写一行原因、完成时挂一条不打断操作的提示。出错（动过 ref 之后才失败）照常进错误条，
   * 文案里写着此刻的状态。
   */
  /** 返回这次有没有跟进成（`done`）：成了，调用方在 `running` 放开之后叫醒一次（`nudgeAutoFollow`）。 */
  const autoFollow = async (key: string): Promise<boolean> => {
    // 又检测到一次新的 SVN 变化：不管这次自动不自动，上一次的提示都过时了
    d.clearAutoFollowed()
    // 先看开关：关着时横幅照旧，连 merge driver 的原因也不该写（review 中-2）
    if (!(await d.autoFollowEnabled())) {
      d.setAutoFollowSkip(null)
      return false
    }
    const risk = await d.mergeRiskForAuto()
    if (risk) {
      d.setAutoFollowSkip({ key, message: risk })
      return false
    }
    d.setFollowing(true)
    let outcome: AutoFollowOutcome
    try {
      outcome = await perfSpan('autoFollow', async () => {
        let since = d.svnSyncMark()
        let run = await timed(() => svnFollowAuto(d.repoPath))
        // 暂时性的原因（外部 git 刚好拿着索引锁、另一次同步刚好在跑）：同一个现场只试一次，不重试的话
        // 就再也不自动了（review 中-1）。等一下再试一次，还不行才写进横幅
        if (run.value.kind === 'skipped' && TRANSIENT_SKIPS.has(run.value.code)) {
          await d.wait(TRANSIENT_RETRY_MS)
          since = d.svnSyncMark()
          run = await timed(() => svnFollowAuto(d.repoPath))
        }
        if (run.value.kind !== 'done') return run.value
        // 提示照旧等刷新完再挂：它不挡操作，早挂出来反而会和还没撤的「有新内容」横幅叠在一起
        await afterFollow(run.value.sync, since)
        return { ...run.value, report: { ...run.value.report, elapsedMs: run.ms } }
      })
    } catch (err) {
      d.reportError(err)
      return false
    } finally {
      d.setFollowing(false)
    }
    if (outcome.kind === 'skipped') {
      d.setAutoFollowSkip(
        outcome.code === AUTO_FOLLOW_OFF
          ? null
          : { key, message: outcome.message, conflict: outcome.conflict },
      )
      // 原因在工作副本里（SVN 冲突）：处理掉之后现场键不变，不放掉就再也不自动了。
      // 期间记下的已经是别的现场时不动它
      if (RECHECK_SKIPS.has(outcome.code) && d.autoTried.current === key) d.autoTried.current = null
      return false
    }
    const { left, pin } = await sceneLeftBy(outcome.report)
    d.setHeldBack(pin)
    if (left) {
      // 只剩留下的文件：再试一次结论相同，直接把这个现场记成试过，原因写给横幅
      d.autoTried.current = left.key
      d.setAutoFollowSkip(left)
    } else {
      d.setAutoFollowSkip(null)
    }
    perfMark('autoFollow.done')
    d.showAutoFollowed(outcome.report)
    return true
  }

  /**
   * 部分跟进做完、刷新之后的现场是不是只剩它留下的文件（`heldBackScene`），以及把留下的文件钉在
   * 这个现场上（`pinHeldBack`）。整次跟进时不去读。
   * 读不出来就当不是：只是少省一次，下一次判定写回时照常自动试；留下的文件照样记下，不按现场比。
   */
  const sceneLeftBy = async (report: FollowReport) => {
    if (!report.heldBack?.length) return { left: null, pin: null }
    try {
      const scene = await d.sceneAfterFollow()
      return {
        left: heldBackScene(report.heldBack, scene.sync, scene.head),
        pin: pinHeldBack(report.heldBack, scene),
      }
    } catch (err) {
      console.warn('读部分跟进之后的现场失败：', err)
      return { left: null, pin: pinHeldBack(report.heldBack, null) }
    }
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
        d.clearAutoFollowed()
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
      // 用盒子的理由同 `svnUpdate`。`then` 排在 `exclusive` 的 finally 之后：叫醒时 `running` 已经放开
      const box = { done: false }
      void exclusive(async () => {
        box.done = await autoFollow(key)
      }).then(() => {
        if (box.done) d.nudgeAutoFollow()
      })
      return true
    },
    // 没有单独的「手动跟进」入口（Review 3 第 4 条）：它只经由跟进冲突的确认框与「切换并手动跟进」
    // 发起，两处都已在 `exclusive` 里，见 `settleFollow` 与 `checkoutBehind`
    handleCheckoutBehind: (branch: string) => exclusive(() => checkoutBehind(branch)),
    handleSyncAction: (id: SyncActionId | FollowActionId) =>
      exclusive(() => (id === 'followContinue' ? followContinue() : syncAction(id))),
  }
}
