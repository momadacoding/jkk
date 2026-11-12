import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useCallback, useEffect, useRef, useState, type MutableRefObject } from 'react'
import type { CommitNode, ConflictInfo, DiffData, GraphData, PullRequest, RefPointer, RepoInfo, StashEntry, StatusData, UpstreamInfo, Worktree } from '../engine/types'
import { ALL_CAPABILITIES, type Capabilities } from '../engine/capabilities'
import {
  auditIsDue,
  isPlainCommitOn,
  markOnBlur,
  recordAfterSuccess,
  runSvnSyncCheck,
  sameHeadForSync,
  SVN_AUDIT_POLL_MS,
  SVN_TRAILING_RECHECK_MS,
  type EolStatus,
  type HeadObservation,
  type ObservationMemo,
  type ServerAudit,
  type SyncStatus,
  type SyncVerdict,
  type WcDbStamp,
  type WcDbStampMemo,
} from '../engine/svnSync'
import {
  needsLockPolling,
  SYNC_LOCK_POLL_MS,
  type SyncState,
  type SyncStateUnreadable,
} from '../engine/svnJournal'
import {
  baselineMap,
  remapRefs,
  selectionDiffScope,
  spliceHistory,
  type SvnRevNode,
} from '../engine/svnGraph'

import { toErrorCode, toErrorMessage } from '../engine/errors'
import { createLatestGate } from '@/engine/latestGate'
import { createSingleFlight } from '@/engine/singleFlight'
import { createSvnCheckQueue } from '@/engine/svnCheckQueue'
import { applyStatusPatch, type StatusPatch } from '@/engine/statusPatch'
import { changesDiffScope, isChangesFileScope, type FileSelection } from '@/engine/fileSelection'
import { backendDiffOptions, diffOptionsKey, type BackendDiffOptions } from '@/engine/diffPrefs'
import { getDiffPrefs, useDiffOptionsKey } from './diffPrefsStore'
import type { BlameData, FileHistory } from '@/engine/fileHistory'
import type { DriverReport } from '@/engine/mergeDrivers'
import type {
  Health,
  InitPreflight,
  InitReport,
  PinnedExternal,
  RebuildReport,
} from '../engine/svnInit'
import { samePath } from '../engine/path'
import type {
  AutoFollowOutcome,
  FollowOutcome,
  RestoreOutcome,
  SvnUserInfo,
} from '../engine/svnFollow'
import type { HookHealth, TsvnHooksStatus } from '../engine/tsvnHooks'
import type { SyncInExpect, SyncInOutcome, SyncInPreview } from '../engine/svnSyncIn'
import type { SyncOutExpect, SyncOutOutcome, SyncOutPreview } from '../engine/svnSyncOut'
import { nextSelection } from '../engine/selection'

/** 首屏与每次「加载更多」拉多少条（spec §10.2：不做无限滚动）。 */
const SVN_LOG_PAGE = 200

/**
 * 把 SVN 序列接进 git 历史——**只在 SVN 模式下**。
 *
 * # 为什么普通 git 仓库必须原样返回
 *
 * `spliceHistory` 会把带 `git-svn-id` trailer 的 commit 当成 baseline
 * **从图上删掉**（spec §10.3 ③：它在图上的位置由 SVN 序列决定）。
 *
 * 而一个从 `git svn` 迁移过来的**普通 git 仓库**，历史里遍地都是这种 trailer
 * ——那是 `git svn` 自己写的（spec §6.4.1 就是照它的格式设计的）。无条件接合的话，
 * 这类仓库一打开就会发现大半历史凭空消失，而 svnRevs 是空的、没有任何东西接住它们。
 *
 * 所以判据必须是「这个仓库是不是 SVN 混合仓库」，而不是「svnRevs 空不空」。
 */
function merge(
  s: RepoState,
  gitCommits: CommitNode[],
  svnRevs: SvnRevNode[],
  refs: RefPointer[] = s.refs,
): CommitNode[] {
  if (s.capabilities.vcs !== 'svn') return gitCommits
  // ref 与 HEAD 是「已交付的提交」判据里的起点：还被它们指着的提交照常显示。
  // `s.refs` 里指向基线的已经重映射成 `svn:N`，不影响——判据只看非基线提交
  return spliceHistory(svnRevs, gitCommits, [...refs.map((r) => r.hash), s.headHash])
}

export interface RepoState {
  path: string | null
  /** 分支名，显示用。 */
  head: string
  /** HEAD 的完整 oid，工作区伪行挂在它上面。别和 head 混用。 */
  headHash: string
  commits: CommitNode[]
  refs: RefPointer[]
  worktrees: Worktree[]
  upstream: UpstreamInfo | null
  stashes: StashEntry[]
  /**
   * 当前仓库的 open PR。空数组既表示「没有 PR」也表示「这台机器读不到」——
   * 两者在界面上是同一件事：分支旁边不画徽章。
   */
  pullRequests: PullRequest[]
  /**
   * 当前后端支持哪些功能。组件据此**不渲染**对应入口。
   *
   * 默认（也是读取失败时的兜底）是全开：普通 git 仓库占绝大多数，
   * 把它们因为一次后端读取失败搞成半残，比在 SVN 仓库上多露一个入口更糟。
   */
  capabilities: Capabilities
  status: StatusData
  /**
   * 冲突文件的分类（`read_conflicts`），跟着 `status` 刷新。
   *
   * `null` 表示还没读到或读取失败：冲突文件只显示「这个文件有冲突」，不给操作。
   * 不能把失败当成空数组——那样界面会以为这些文件不是冲突。
   */
  conflicts: ConflictInfo[] | null
  /**
   * 本机 merge driver 配置检查（spec §4.6）。`null` = 还没读到，不挂横幅。
   * 打开仓库和每次 `repo-changed` 都刷新；后端有缓存，`.gitattributes` 没变时很便宜。
   */
  mergeDrivers: DriverReport | null
  diff: DiffData
  /** `diff` 是按哪个 scope 读出来的；`null` = 还没读过。显示前用 `currentDiff` 与当前视图要的比对。 */
  diffScope: string | null
  selectedId: string | null
  loading: boolean
  error: string | null
  /**
   * 上一次失败的错误码；`null` 表示没失败、或那个异常不带 code。
   *
   * 与 `error` 分开存：`error` 是给人看的消息，这个是给**代码**分流用的。
   * 目前唯一的消费方是 `svn_not_initialized` —— 「有 `.svn` 无 `.git`」不是错误，
   * 是一个还没走的流程（spec §6.1），该显示初始化向导而不是一条红色横幅。
   */
  errorCode: string | null
  /**
   * 被 `-r` 钉死、且与白名单有交集的 external。非空即处于 pinned 模式。
   *
   * spec §6.5.3 第 3 条要求这条残余风险**UI 常驻说明**——只在初始化向导里显示
   * 一次不算常驻，用户下次打开这个仓库就看不到了。所以它进 store，每次打开都读。
   */
  pinned: PinnedExternal[]
  /**
   * 失同步的完整校验结果（spec §6.6.3 的**第二档**）。`null` = 还没读到。
   *
   * `null` 与「同步」是两回事：打开仓库的那一瞬间它是 `null`，那时挂任何横幅
   * 都等于每次打开都闪一下。`describeSync(null)` 因此返回 `null`。
   */
  svnSync: SyncStatus | null
  /**
   * 最近一次失同步判定报了 `svn_no_baseline`：基线分支 `svn/base` 不存在（第 70 条）。
   * 只在判定报这个错时置位，别的失败与成功都不动它。读它的两处（`describeMissingBaseline`、
   * `needsStandaloneCheck`）都同时看 `svnSync`：有结论就说明基线读得到，标记作废。
   */
  svnBaselineMissing: boolean
  /** 服务器侧清单核对（§6.3.1 / NB-2）。走网络，失败静默。 */
  svnAudit: ServerAudit | null
  /**
   * 行尾规则装了没有（spec §7.3.0）。`null` = 还没读到，**不等于**「不需要迁移」
   * ——读失败时也是 `null`，那时不挂横幅，但同样不能当成「已经迁移过」。
   */
  svnEol: EolStatus | null
  /**
   * 同步锁与上次未收尾的 journal（spec §5.1、§7.6）。`null` = 还没读到。
   *
   * 读失败**不写 `null`**（与 `svnEol` 不同）：写成 `{ readError }`，横幅照样
   * 挂出来并让其余横幅让位。`null` 不挂横幅，那等于把「journal 坏了」显示成
   * 「上次干净收尾了」。
   */
  svnSyncState: SyncState | SyncStateUnreadable | null
  /** 落后基线的本地分支（分析 §6.11.3）。分支列表打标记，切过去时要确认。 */
  svnBranchesBehind: string[]
  /** SVN 历史（缓存 + 已拉取的），按 revision 降序。 */
  svnRevs: SvnRevNode[]
  /**
   * 后端 `load_graph` 原样给的 git 提交，**未接合**。
   *
   * `commits` 是接合后的结果；两者分开存是因为 svnRevs 与 gitCommits 各自会被
   * 异步刷新，任一变化都要重新接一次，而接合是纯函数，重算比增量维护更不容易错。
   */
  gitCommits: CommitNode[]
  /**
   * 已经翻到最早的 revision 了，「加载更多」该禁用。**由后端算，这里只转发。**
   *
   * 判据是「每个 target 的**连续覆盖区间**都到了 r1」，而覆盖区间只存在于后端
   * 缓存里。前端手上的 `svnRevs` 是所有 target ∪ 孤儿锚点去重后的列表，
   * 既看不出连续性、也分不清哪条属于哪个 target，用它推断完整性怎么写都是错的
   * （2026-09-16 独立 review：原先「这一页没带回新东西 = 到底了」的写法，
   * 配上同样取自并集的翻页游标，会在中间还差几百个 revision 时就把按钮禁掉）。
   */
  svnLogComplete: boolean
}

const initialState: RepoState = {
  path: null,
  head: '',
  headHash: '',
  commits: [],
  refs: [],
  worktrees: [],
  upstream: null,
  stashes: [],
  pullRequests: [],
  capabilities: ALL_CAPABILITIES,
  status: { staged: [], unstaged: [] },
  conflicts: null,
  mergeDrivers: null,
  diff: { files: [] },
  diffScope: null,
  selectedId: null,
  loading: false,
  error: null,
  errorCode: null,
  pinned: [],
  svnSync: null,
  svnBaselineMissing: false,
  svnAudit: null,
  svnEol: null,
  svnSyncState: null,
  svnBranchesBehind: [],
  svnRevs: [],
  gitCommits: [],
  svnLogComplete: false,
}

/**
 * 读取后端能力，失败时退回全开。
 *
 * 为什么不让失败冒泡：`read_capabilities` 走的是 `backend_for`，而打开仓库那条
 * 路径上 `open_repo` 已经先跑过同一个 `backend_for` 了——真的打不开的仓库在那里
 * 就报错了。能走到这里还失败的，只剩「命令没注册」这类本项目自己的 bug，
 * 把普通 git 仓库因此搞成半残不值当。
 */
async function readCapabilities(path: string): Promise<Capabilities> {
  try {
    return await invoke<Capabilities>('read_capabilities', { path })
  } catch (err) {
    console.warn('读取后端能力失败，按普通 git 仓库渲染全部入口：', err)
    return ALL_CAPABILITIES
  }
}

// ---------------------------------------------------------------------------
// SVN 混合仓库（Phase 1b）
//
// 这四个是**薄壳**：不吞错误、不重试、不做判断。三个会改磁盘的命令失败时必须
// 原样抛出去，让向导留在原地把错误显示给用户——`readCapabilities` 那种「失败就
// 退回默认值」的写法在这里是灾难性的（用户会以为建库成功了）。
//
// ⚠️ 参数名是 `snake_case` → `camelCase` 转换后的形态，没有编译期检查
// （AGENTS.md §7.3）。Rust 侧 `path` / `whitelist` / `extensions` 三个都是单词，
// 转换后不变，但别因此以为这层不存在。
// ---------------------------------------------------------------------------

/** 向导第 2 步：P1–P4 加起作用的 `.gitignore`，什么都不创建。 */
export function svnPreflightCheck(
  path: string,
  whitelist: string[],
  extensions: string[],
): Promise<InitPreflight> {
  return invoke<InitPreflight>('svn_preflight_check', { path, whitelist, extensions })
}

/**
 * 走完初始化向导。**会在工作副本里创建 `.git`**；不尊重时还会把起作用的 `.gitignore`
 * 改名停用。任何一步失败后端会自己删干净、改回原名。
 *
 * ⚠️ Rust 侧参数名是 `respect_gitignore`，这里必须写成 `respectGitignore`（AGENTS.md §7.3）。
 */
export function svnInitHybrid(
  path: string,
  whitelist: string[],
  extensions: string[],
  respectGitignore: boolean,
  acknowledged: string[],
): Promise<InitReport> {
  return invoke<InitReport>('svn_init_hybrid', {
    path,
    whitelist,
    extensions,
    respectGitignore,
    acknowledged,
  })
}

/** 重建基线。**会移动基线指针**；不动 HEAD、不动索引、不动磁盘。 */
export function svnRebuildBaseline(path: string): Promise<RebuildReport> {
  return invoke<RebuildReport>('svn_rebuild_baseline', { path })
}

/**
 * 体检当前基线。
 *
 * 三种结果都从 `Promise` 正常返回，**只有命令本身没跑起来才 reject**：
 * 「没能验证」与「验证不通过」必须分开（spec §6.8.6）。
 */
export function svnCheckBaseline(path: string): Promise<Health> {
  return invoke<Health>('svn_check_baseline', { path })
}

/**
 * 当前仓库里被钉死、且与白名单有交集的 external。非混合仓库返回空列表。
 *
 * ⚠️ 后端要跑 `svn propget -R`，在几万文件的工作副本上不便宜。
 * 调用方**不要 await**。
 */
export function svnPinnedExternals(path: string): Promise<PinnedExternal[]> {
  return invoke<PinnedExternal[]>('svn_pinned_externals', { path })
}

/**
 * 用户表明要删远程时调：后台先问好远程默认分支（删除护栏要用）。
 *
 * 这次 `ls-remote` 是一整趟 SSH 握手，实测到 GitHub 3 秒多。放在用户读弹框的
 * 时间里做，点确认时就只剩 push 一次连接。**只是提速**：护栏只认成功的结果，
 * 预取失败或没回来，它会限时等待后自己再问，所以这里吞掉一切错误，调用方不必 await。
 *
 * 删除弹框已改用 {@link probeRemoteBranch}（同一次查询顺带问分支在不在）。
 */
export function prefetchRemoteDefault(path: string, remote: string): void {
  invoke('prefetch_remote_default', { path, remote }).catch((err) => {
    console.warn('预取远程默认分支失败：', err)
  })
}

/**
 * 用户勾上「顺便删远程」时调：远程上还有没有这条分支。`null` 表示问不到。
 *
 * 顺带替删除护栏问好远程默认分支：这次 `ls-remote` 是一整趟 SSH 握手，实测到 GitHub
 * 3 秒多，放在用户读弹框的时间里做，点确认时就只剩 push 一次连接。护栏只认成功的结果，
 * 这里失败或没回来，它会限时等待后自己再问。
 * **只读**：过期的跟踪 ref 等删除做完后用 {@link forgetGoneRemoteBranch} 清。
 * 这只是提前告知：删除那一步对「远程已经没有」本来就按成功处理，所以出错一律当问不到。
 */
export function probeRemoteBranch(
  path: string,
  remote: string,
  branch: string,
): Promise<boolean | null> {
  return invoke<boolean | null>('probe_remote_branch', { path, remote, branch }).catch((err) => {
    console.warn('探测远程分支失败：', err)
    return null
  })
}

/**
 * 探测答了「远程上已经没有」、确认的删除做完之后调：清掉本地过期的跟踪 ref。
 * 只读本地配置、不走网络；push 目标不是 fetch 侧时后端什么都不做。
 */
export function forgetGoneRemoteBranch(path: string, remote: string, branch: string): Promise<void> {
  return invoke<void>('forget_gone_remote_branch', { path, remote, branch })
}

/** 同 {@link prefetchRemoteDefault}，按侧边栏的 `origin/feat/x` 形式给出（拆远程名在后端做）。 */
export function prefetchRemoteDefaultForRef(path: string, remoteBranch: string): void {
  invoke('prefetch_remote_default_for_ref', { path, remoteBranch }).catch((err) => {
    console.warn('预取远程默认分支失败：', err)
  })
}

/**
 * 只跑远程护栏，不删任何东西：要删的分支是默认分支时 reject。
 * 删除序列的第一步不可撤销时先调它，理由见 `runBranchDeletion`。
 */
export function checkRemoteDeletable(path: string, remote: string, branch: string): Promise<void> {
  return invoke<void>('check_remote_deletable', { path, remote, branch })
}

/** 行尾规则装了没有（spec §7.3.0）。**纯读**，不走网络。 */
export function svnEolStatus(path: string): Promise<EolStatus> {
  return invoke<EolStatus>('svn_eol_status', { path })
}

/**
 * 同步锁与上次未收尾的 journal（spec §5.1、§7.6）。**纯读**，不走网络，也不拿锁
 * ——它恰恰是用来观察锁的。
 */
export function svnSyncState(path: string): Promise<SyncState> {
  return invoke<SyncState>('svn_sync_state', { path })
}

export function svnSyncInPreview(path: string): Promise<SyncInPreview> {
  return invoke<SyncInPreview>('svn_sync_in_preview', { path })
}

/** `expect` 的字段名是 camelCase，与后端 `SyncInExpect` 的 serde 约定对齐（AGENTS.md §7.3）。 */
export function svnSyncIn(path: string, expect: SyncInExpect): Promise<SyncInOutcome> {
  return invoke<SyncInOutcome>('svn_sync_in', { path, expect })
}

export function svnSyncContinue(path: string): Promise<SyncInOutcome> {
  return invoke<SyncInOutcome>('svn_sync_continue', { path })
}

export function svnSyncAbort(path: string): Promise<SyncInOutcome> {
  return invoke<SyncInOutcome>('svn_sync_abort', { path })
}

/** 跟进外部提交（分析 §6.5）：录新基线、对象层重放分支、发布。不写磁盘，不发 SVN 写命令。 */
export function svnFollow(path: string): Promise<FollowOutcome> {
  return invoke<FollowOutcome>('svn_follow', { path })
}

/**
 * 自动跟进（事故分析 6.12）：条件全部满足才发布，否则 `skipped`、一个 ref 都不动。设置里关掉时
 * 后端直接返回 `skipped`（`AUTO_FOLLOW_OFF`）。
 */
export function svnFollowAuto(path: string): Promise<AutoFollowOutcome> {
  return invoke<AutoFollowOutcome>('svn_follow_auto', { path })
}

/** 把崩溃留下的跟进走完，或回到一致状态后重新跟进。 */
export function svnFollowContinue(path: string): Promise<FollowOutcome> {
  return invoke<FollowOutcome>('svn_follow_continue', { path })
}

/** 手动跟进：跟进冲突时的兜底，会写磁盘。冲突之后的继续 / 放弃走 `svnSyncContinue` / `svnSyncAbort`。 */
export function svnFollowManual(path: string): Promise<SyncInOutcome> {
  return invoke<SyncInOutcome>('svn_follow_manual', { path })
}

/**
 * 跟进报告里的「放回 Local-Only 补丁」（使用报告 9.1）：把 `paths` 写回成分支的版本。后端锁内
 * 逐个重验条件，不满足的跳过并给出原因；会写磁盘，调用方随后刷新状态。
 */
export function svnRestoreLocalOnly(path: string, paths: string[]): Promise<RestoreOutcome> {
  return invoke<RestoreOutcome>('svn_restore_local_only', { path, paths })
}

/** 落后基线的本地分支。纯本地读；非混合仓库或没有基线时返回空列表。 */
export function svnBranchesBehind(path: string): Promise<string[]> {
  return invoke<string[]>('svn_branches_behind', { path })
}

/** 「我的 SVN 用户名」：配置的值与自动探测的结果。 */
export function svnGetUser(path: string): Promise<SvnUserInfo> {
  return invoke<SvnUserInfo>('svn_get_user', { path })
}

/** 写配置里的 `svnUser`，空串清除。 */
export function svnSetUser(path: string, user: string): Promise<void> {
  return invoke<void>('svn_set_user', { path, user })
}

/** TortoiseSVN 钩子的状态与注册计划（C1h）。只读注册表，会跑 `svn info` 找工作副本根。 */
export function tsvnHooksStatus(path: string): Promise<TsvnHooksStatus> {
  return invoke<TsvnHooksStatus>('tsvn_hooks_status', { path })
}

/** 写 TortoiseSVN 的钩子设置。调用方先让用户在注册框里确认。 */
export function tsvnHooksRegister(path: string, blockLeaks: boolean): Promise<HookHealth> {
  return invoke<HookHealth>('tsvn_hooks_register', { path, blockLeaks })
}

/** 删掉本仓库的 jkk 钩子，别人的钩子原样保留。 */
export function tsvnHooksUnregister(path: string): Promise<void> {
  return invoke<void>('tsvn_hooks_unregister', { path })
}

export function svnSyncOutPreview(path: string): Promise<SyncOutPreview> {
  return invoke<SyncOutPreview>('svn_sync_out_preview', { path })
}

/** `expect` 的字段名是 camelCase，与后端 `SyncOutExpect` 的 serde 约定对齐（AGENTS.md §7.3）。 */
export function svnSyncOut(path: string, expect: SyncOutExpect): Promise<SyncOutOutcome> {
  return invoke<SyncOutOutcome>('svn_sync_out', { path, expect })
}

export function svnSyncOutContinue(path: string): Promise<SyncOutOutcome> {
  return invoke<SyncOutOutcome>('svn_sync_out_continue', { path })
}

export function svnSyncOutAbort(path: string): Promise<SyncOutOutcome> {
  return invoke<SyncOutOutcome>('svn_sync_out_abort', { path })
}

/**
 * 走一次行尾迁移：装规则 + 重录基线。
 *
 * **会移动基线指针**，与 `svnRebuildBaseline` 同等分量，调用前必须先弹确认。
 * 重建失败时后端会把规则还原，仓库回到原状。
 */
export function svnMigrateEol(path: string): Promise<RebuildReport> {
  return invoke<RebuildReport>('svn_migrate_eol', { path })
}

/** 完整失同步校验（spec §6.6.3 第二档）。纯本地读，不走网络。 */
export function svnSyncStatus(path: string): Promise<SyncStatus> {
  return invoke<SyncStatus>('svn_sync_status', { path })
}

/** 记一份失焦快照（只看索引与 HEAD）。非混合仓库什么也不做。 */
export function svnFollowMark(path: string): Promise<void> {
  return invoke<void>('svn_follow_mark', { path })
}

/**
 * 各工作副本 `wc.db` 的 mtime + size（分析 §6.6），§6.6.3 的第一档：与上次观测值比，变了才跑第二档。
 * wc.db 没变时后端只有几次 `stat`；变了时后端重新枚举工作副本（每个一次 `svn info` + `svn status`）。
 * 非混合仓库返回空列表。
 */
export function svnWcDbStamps(path: string): Promise<WcDbStamp[]> {
  return invoke<WcDbStamp[]>('svn_wc_db_stamps', { path })
}

/** 一页 SVN 历史 + 「还能不能再往下翻」，对应后端 `LogPage`。 */
export interface SvnLogPage {
  revs: SvnRevNode[]
  complete: boolean
}

/** 读缓存里的 SVN 历史，**不走网络**。打开仓库时先用它把图画出来。 */
export function svnReadLog(path: string): Promise<SvnLogPage> {
  return invoke<SvnLogPage>('svn_read_log', { path })
}

/**
 * 拉取 SVN 历史，**走网络**。`older = true` 是往下翻一页。
 *
 * ⚠️ 翻页游标**不在这里传**：它由后端按各 target 自己的连续覆盖区间算
 * （见 Rust 侧 `fetch_log` 的文档）。这里只说「再往下翻一页」。
 */
export function svnFetchLog(path: string, older: boolean, limit: number): Promise<SvnLogPage> {
  return invoke<SvnLogPage>('svn_fetch_log', { path, older, limit })
}

/** 服务器侧清单核对（§6.3.1 / NB-2）。**走网络**，可能不便宜。 */
export function svnServerAudit(path: string): Promise<ServerAudit> {
  return invoke<ServerAudit>('svn_server_audit', { path })
}

/** 这个文件改动过的提交（`git log --follow`，跨重命名），从新到旧。`revs` 是起点，取并集。 */
export function readFileHistory(path: string, file: string, revs: string[]): Promise<FileHistory> {
  return invoke<FileHistory>('file_history', { path, file, revs })
}

/** 逐行 Blame。`rev` 为 null 时 blame 工作区里的文件，没提交的行归到全 0 的 sha。 */
export function readBlame(path: string, file: string, rev: string | null): Promise<BlameData> {
  return invoke<BlameData>('blame_file', { path, file, rev })
}

/**
 * 一个文件在某次提交里的 diff（对第一个父提交）。改名那一条要带上 `oldFile` 才配得成对。
 * `options` 是 diff 面板的上下文行数 / 忽略空白（`backendDiffOptions`）。
 */
export function readCommitFileDiff(
  path: string,
  rev: string,
  file: string,
  oldFile: string | null,
  options: BackendDiffOptions,
): Promise<DiffData> {
  return invoke<DiffData>('commit_file_diff', { path, rev, file, oldFile, options })
}

/** 当前 diff 面板开关对应的后端选项和它的键。 */
function currentDiffOptions(): { options: BackendDiffOptions, key: string } {
  const options = backendDiffOptions(getDiffPrefs())
  return { options, key: diffOptionsKey(options) }
}

/**
 * 每个标签的 store 都登记在这里，收「本应用在某个仓库写了 index」的通知（`afterIndexWrite`）。
 * 所有标签在同一个页面里，进程内通知就够了，不必绕 Tauri 事件。
 */
const indexWriteSubscribers = new Set<(path: string, origin: object) => void>()

export function useRepoStore() {
  /** 这个 store 的身份：通知里认出「是自己发的」。 */
  const storeIdentity = useRef({}).current
  const [state, setState] = useState<RepoState>(initialState)
  /** 上一次校验成功时的 HEAD。见 `checkSvnSyncIfHeadMoved`。放 ref 的理由同 `lastWcDbStamps`。 */
  const lastCheckedHead = useRef<ObservationMemo<HeadObservation>>({ value: null })
  /** 已经为它请求了强制校验、还没落定的 HEAD。见 `checkSvnSyncIfHeadMoved`。 */
  const headRequested = useRef<HeadObservation | null>(null)
  /**
   * 本应用叠在还没落定的那一轮（`on`，就是当时的 `headRequested` 那个对象）上的普通提交（`noteOwnCommit`）。
   * 那一轮写回后直接记成它，不再为它排一轮（独立 review 第 8 条）；没写回就替它补一轮（第二轮第 1 条）。
   */
  const ownCommitOnPending = useRef<{ on: HeadObservation, head: HeadObservation } | null>(null)
  /**
   * §6.6.3 第一档的「上次观测值」：最近一次写回了结论的校验开跑之前读到的各 wc.db 戳。
   * 文件变动、焦点检测、打开仓库共用，失焦快照也按它判断。
   *
   * **放 ref 不放 state**：读它的是 `repo-changed` 监听器，而那个 `useEffect`
   * 的依赖是 `[state.path, state.capabilities.vcs]`。放进 state 的话监听器会**永远闭包住
   * 注册那一刻的值**（`null`），比较恒为不等，于是第二档的全量校验
   * 每次文件变动都跑一遍——分层就白做了。这是独立 review 抓到的。
   *
   * 进程重启后是 `null`，第一次检测当作「变了」跑一次全量。盒子里那个 `value`
   * **什么时候可以写**是 `svnSync.ts` 里 `runSvnSyncCheck` / `recordAfterSuccess` 的事：
   * 只有第二档真的跑成功、写回了才记。
   */
  const lastWcDbStamps = useRef<WcDbStampMemo>({ value: null })
  /**
   * 最近一次第二档校验的结论，失败时是 `null`。失焦快照要它。
   *
   * 不读 `state.svnSync`：失焦的处理函数闭包里那份是注册时的，就算走 ref 同步也要等一次渲染，
   * 校验结果写回与失焦之间没有先后保证。
   */
  const latestVerdict = useRef<SyncVerdict['kind'] | null>(null)
  /** 最近一次写回的完整结论，同步写。提交前的「先跟进」提醒要它（事故分析 6.6），理由同上。 */
  const latestSync = useRef<SyncStatus | null>(null)
  /**
   * `openRepo` 写进 state 的那个路径，同步镜像一份。
   *
   * `latestVerdict` 写回前要核对「还是不是这个仓库」（第 71 条），但不能放进 `setState` 的
   * updater 里核对：updater 可能要等下一次渲染才执行，而 `trackSvnCheck` 在那之前就把
   * 在跑的计数归零了，失焦快照会拿上一次的结论去写（PR 113 review）。
   */
  const openedPath = useRef<string | null>(null)
  /** 正在跑的校验（含读戳、读 revision 的那一段）。见 `shouldMarkOnBlur`。 */
  const svnChecksRunning = useRef(0)

  /**
   * 上一次**开始**服务器侧核对的时刻，与 `auditInFlight` 一起构成
   * `auditIsDue` 的输入。用 ref 而不是 state：写它不该引起重渲染，
   * 而且定时器闭包读 state 会永远读到注册那一刻的值（`lastWcDbStamps`
   * 的注释里记的是同一个坑）。
   */
  const lastAuditAt = useRef(0)
  const auditInFlight = useRef(false)

  /** `refreshConflicts` 的请求代次，见 `createLatestGate`。放 ref：要跨渲染保持计数。 */
  const refreshGate = useRef(createLatestGate()).current
  /**
   * `refreshStatus` / `refreshDiff` 同一种读取同时只跑一轮，见 `createSingleFlight`。
   * 放 ref：在跑的那一轮要跨渲染记住。
   */
  const refreshFlights = useRef(createSingleFlight()).current
  /** SVN 失同步检测的单飞队列，见 `requestSvnCheck`。放 ref 的理由同上。 */
  const svnCheckQueue = useRef(createSvnCheckQueue()).current
  /**
   * 最后一次请求状态 / diff 的仓库路径。单飞按路径分开（不同仓库的读取不能互相顶替），
   * 旧仓库晚到的结果就要靠它挡住，不让它盖掉新仓库的。现在每个标签页的路径是固定的，
   * 但 store 不该依赖这一点（独立 review）。
   */
  const lastStatusPath = useRef<string | null>(null)
  const lastDiffPath = useRef<string | null>(null)
  /**
   * 排队等下一轮状态读取的需求，按仓库路径分开：要不要全量、要局部重读哪些路径。
   * 见 `refreshStatus` 为什么不能交给单飞的「最后一个任务」决定。
   */
  const statusWanted = useRef(new Map<string, { full: boolean, paths: Set<string> }>())
  /**
   * 每个仓库最后一次读到的状态，局部重读在它上面合并（`applyStatusPatch`）。
   * 只在单飞里写：同一个仓库的读取串行，它就总是最后开读的那一轮的结果。
   */
  const knownStatus = useRef(new Map<string, StatusData>())
  /** 写操作之后的刷新（`refreshChanges`）开始的次数，`refreshDiff` 据此判断在跑的那轮能不能复用。 */
  const writeSeq = useRef(0)
  /**
   * 每个 diff 单飞队列里正在跑的那一轮：读的是哪个 scope、带的哪份后端选项（`diffOptionsKey`）、
   * 开读时的 `writeSeq`、读完的信号。
   */
  const diffRunning = useRef(new Map<string, { scope: string, optionsKey: string, seq: number, done: Promise<void> }>())
  /**
   * 当前视图要显示哪一份 diff，由 `useRepoEffects` 每次渲染写入。
   *
   * `undefined` 表示还不知道（刚打开仓库），此时照常写回。用户操作（stage、discard、commit）的
   * 回调会固定刷新 `workdir`，它可能在用户切到某个提交之后才发出；不核对 scope 的话，
   * 它要么让那个提交的结果作废，要么晚到后把工作区的改动盖在提交详情上（Codex review 第 2 条）。
   */
  const wantedDiffScope = useRef<string | null | undefined>(undefined)

  const openRepo = useCallback(async (path: string) => {
    setState((s) => ({ ...s, loading: true, error: null, errorCode: null }))
    try {
      const info = await invoke<RepoInfo>('open_repo', { path })
      // 每次打开都重新读，且**必然写回**：上一个仓库是 SVN 时留下的能力集合
      // 会把下一个普通 git 仓库的入口一起关掉。
      const caps = await readCapabilities(path)
      openedPath.current = path
      setState((s) => ({
        ...s,
        path,
        head: info.head,
        headHash: info.headHash,
        capabilities: caps,
        loading: false,
      }))
      // 打开一个 worktree 时想看的是「它在哪」，所以定位到它自己的 HEAD。
      // 这里必须把 headHash 显式传下去——上面那次 setState 是异步的，
      // refreshGraph 里的 updater 读不到刚写进去的值。
      await refreshGraph(path, info.headHash)
      await refreshStatus(path)
      const wts = await invoke<Worktree[]>('read_worktrees_cmd', { path })
      setState((s) => ({ ...s, worktrees: wts }))
      await refreshUpstream(path)
      await refreshStashes(path)
      // 不 await：大仓库第一次要扫全部文件的属性，横幅晚一点出现无所谓
      void refreshMergeDrivers(path).catch((err) => console.warn('merge driver 检查失败：', err))
      // 不 await：这一步要走网络（gh 实测一两秒），让它拖住打开仓库的收尾没有意义。
      // 不支持 PR 的后端连这次 gh 调用都省掉——界面上没有任何地方会用到结果。
      if (caps.pullRequests) void refreshPullRequests(path)
      // 同样不 await：后端要跑 `svn propget -R`，在几万文件的工作副本上是秒级。
      // 常驻说明晚一两秒出现无所谓，但不能让打开仓库卡在这儿（spec §6.5.3）。
      void refreshPinned(path)
      if (caps.vcs === 'svn') {
        // 第二档的完整校验：打开仓库是正确性的闸门之一，必须问到底（spec §6.6.3）。
        // 纯本地读，await 得起。顺带记下 HEAD（`forceCheckAtHead`）：不记的话打开后第一次 HEAD 事件
        // （常常就是第一次提交）会因为记录还是 `null` 再跑一遍，同 wc.db 戳在这里就记的理由。
        // HEAD 在校验前现读：开头读的那份可能已经过时（打开要读图、状态，大仓库上要一会儿）
        await refreshHeadInfo(path)
          .then((head) => forceCheckAtHead(path, head), () => refreshSvnSync(path))
          .catch(() => undefined)
        // 同样是纯本地读，且它决定要不要挂迁移横幅——那条横幅会让失同步横幅
        // 和常驻体检入口都让位，所以必须在第一帧之前读到，否则界面会先挂出
        // 一条随后又被替换掉的横幅（spec §7.3.0）。
        await refreshSvnEol(path)
        // 同理，且优先级更高：未收尾 / 同步进行中时**其余横幅全部让位**
        // （中间态下它们的结论可能是错的，出口还是危险的）。必须在第一帧之前
        // 读到，否则会先闪一条随后被撤掉的横幅。
        await refreshSvnSyncState(path)
        void refreshSvnBranchesBehind(path).catch((err) =>
          console.warn('读取落后基线的分支失败：', err),
        )
        // 先用**缓存**把图画出来，再后台拉新的（spec §10.2）。
        await refreshSvnLog(path).catch(() => undefined)
        // 这两条走网络，照 `refreshPullRequests` 的既有模式：不 await、失败静默。
        void fetchSvnLog(path)
        void refreshSvnAudit(path)
      } else {
        // **必须显式清空**：上一个仓库是 SVN 时留下的横幅与历史，会挂在下一个
        // 完全无关的 git 仓库上（`refreshPinned` 的注释里记过同一个坑）。
        setState((s) => ({
          ...s,
          svnSync: null,
          svnBaselineMissing: false,
          svnAudit: null,
          svnEol: null,
          svnSyncState: null,
          svnBranchesBehind: [],
          svnRevs: [],
          svnLogComplete: false,
          // gitCommits 不用清：refreshGraph 每次都整份覆盖它
        }))
      }
    } catch (err) {
      setState((s) => ({
        ...s,
        loading: false,
        error: toErrorMessage(err),
        errorCode: toErrorCode(err),
      }))
    }
  }, [])

  /**
   * 重新读取 HEAD，返回新的 headHash。
   *
   * checkout 之后必须调用，否则 header 与分支高亮会停在旧分支。返回值让调用方
   * 能判断 HEAD 有没有动——上面那次 setState 是异步的，读 state 拿到的是旧值。
   */
  const refreshHead = async (path: string): Promise<string> => (await refreshHeadInfo(path)).hash

  /** 同 `refreshHead`，另外交出分支名与父提交：SVN 第二档据此判断 HEAD 的变化要不要校验。 */
  const refreshHeadInfo = async (path: string): Promise<HeadObservation> => {
    const info = await invoke<RepoInfo>('open_repo', { path })
    setState((s) => ({ ...s, head: info.head, headHash: info.headHash }))
    return { branch: info.head, hash: info.headHash, parents: info.headParents ?? [] }
  }

  /**
   * 重新加载提交图。
   *
   * `preferredId` 默认只在还没有任何选中时生效，用来把首次打开的仓库定位到它自己的
   * HEAD。不传的话会退回「图上第一条提交」——那是**全局最新**的提交，可能属于
   * 另一条分支，跟你刚打开的这个 worktree 毫无关系。
   *
   * `force` 让 preferredId 压过已有选中，用于 pull / checkout 这类由用户主动点出来、
   * 且移动了 HEAD 的操作。选中判定见 `nextSelection`。
   */
  const refreshGraph = async (path: string, preferredId?: string, force = false) => {
    const data = await invoke<GraphData>('load_graph', { path })
    setState((s) => {
      const commits = merge(s, data.commits, s.svnRevs, data.refs)
      // refs 必须跟着一起重映射：baseline commit 被从节点列表里删掉了，而
      // GraphView 按 `refByHash[node.commit.id]` 渲染标签——不映射的话
      // `svn/base` 匹配不到任何一行，**整个看不见**。
      const refs =
        s.capabilities.vcs === 'svn'
          ? remapRefs(data.refs, baselineMap(data.commits))
          : data.refs
      return {
        ...s,
        gitCommits: data.commits,
        commits,
        refs,
        selectedId: nextSelection(s.selectedId, commits.map((c) => c.id), preferredId, force),
      }
    })
  }

  /**
   * 交回读到的状态：Stage / Unstage 之后选中的文件可能换了区，调用方要按新状态算
   * 接下来读哪个区的 diff（第 65 条）。
   *
   * 文件监听、外部工具退出、用户操作会先后各发一次。同时只读一轮，在读时再来的只补读一次，
   * 交回的状态一定是调用之后才开读的（`createSingleFlight`）。各轮串行、按开读先后到达，
   * 写回不必再核对代次，先读完的那轮也不会被作废。
   *
   * @param touched 刚 Stage / Unstage 的路径。给了就只重读这几个路径（`read_status_paths`，
   *   2026-11-05 排查报告 6.13 方案 A）；不给就全量读。同一轮里只要有一个调用方要全量，就全量
   */
  const refreshStatus = async (path: string, touched?: string[]): Promise<StatusData> => {
    lastStatusPath.current = path
    // 先登记这次要读什么，再排进单飞：补跑那一轮开跑时把排队期间登记的合起来读一次。
    // 不能让补跑「用最后一次传进来的任务」决定：全量刷新排在前、局部重读排在后时，
    // 那样补跑就只读几个路径，排队的外部改动被漏掉
    const want = statusWanted.current.get(path) ?? { full: false, paths: new Set<string>() }
    if (touched) for (const p of touched) want.paths.add(p)
    else want.full = true
    statusWanted.current.set(path, want)
    const status = await refreshFlights.run(`status:${path}`, async () => {
      let read: StatusData
      try {
        read = await readStatus(path)
      } catch (err) {
        // 这一轮没读到，「上一次的状态」就落后于磁盘：它本该反映的改动（比如文件监听那轮要读的保存）
        // 不在里面。下一次局部重读不能在它上面合并，清掉让下一轮走全量（第二轮独立 review 第 1 条）
        knownStatus.current.delete(path)
        throw err
      }
      knownStatus.current.set(path, read)
      if (lastStatusPath.current === path) setState((s) => ({ ...s, status: read }))
      return read
    })
    // 冲突的分类跟着 status 走：文件监听、外部工具退出、用户操作都经过这里，
    // BC 存盘后 ① 变 ② 不需要另挂一条刷新路径。
    // 放在单飞外面：它有自己的代次闸，不该让下一轮状态读取排在它后面
    await refreshConflicts(path, status)
    return status
  }

  /**
   * 单飞里真正的那次读取：取走登记的需求，能局部就局部。
   *
   * 局部重读要在「上一次读到的状态」上合并，所以只在读过这个仓库之后才走；合并发现 git 的
   * 连带改动、或者局部重读本身失败时，退回全量读——列表不能停在 Stage 之前的样子。
   */
  const readStatus = async (path: string): Promise<StatusData> => {
    const want = statusWanted.current.get(path)
    statusWanted.current.delete(path)
    const prev = knownStatus.current.get(path)
    // 暂存区里的 .gitattributes 决定别的文件按什么 eol / text 规则比较：Stage / Unstage 它会让别的文件
    // 在 Unstaged 里出现或消失，而 Staged 里看不出来，合并检测不到（第二轮独立 review 第 3 条）
    const touchesAttributes = [...(want?.paths ?? [])].some(
      (p) => p === '.gitattributes' || p.endsWith('/.gitattributes'),
    )
    if (want && !want.full && want.paths.size > 0 && prev && !touchesAttributes) {
      const touched = [...want.paths]
      try {
        const patch = await invoke<StatusPatch>('read_status_paths', { path, paths: touched })
        const merged = applyStatusPatch(prev, touched, patch)
        if (merged) return merged
      } catch (err) {
        console.warn('局部重读状态失败，改为全量读：', err)
      }
    }
    return invoke<StatusData>('read_status', { path })
  }

  /**
   * 只在 status 里有冲突时才调后端，否则直接置空。
   *
   * 置空前也要先领一个代次：上一份 `read_conflicts` 可能还在路上，冲突在外面解决了之后
   * 它晚到的话会把已经不存在的冲突写回来。
   */
  const refreshConflicts = async (path: string, status: StatusData) => {
    const token = refreshGate.begin('conflicts')
    if (!status.unstaged.some((f) => f.status === 'conflicted')) {
      setState((s) => ({ ...s, conflicts: [] }))
      return
    }
    try {
      const conflicts = await invoke<ConflictInfo[]>('read_conflicts', { path })
      if (!refreshGate.isLatest(token, 'conflicts')) return
      setState((s) => ({ ...s, conflicts }))
    } catch (err) {
      // 读不到就退回「还没读到」，界面不给任何操作；错误照常交给调用方报出来
      if (refreshGate.isLatest(token, 'conflicts')) setState((s) => ({ ...s, conflicts: null }))
      throw err
    }
  }

  /**
   * 重新检查 merge driver 并写回横幅。返回这次的结果：合并前的确认要用**刚查到的**，
   * 不能用 state 里可能是几秒前的那份。`samePath` 守卫同 `refreshSvnEol`。
   */
  const refreshMergeDrivers = async (path: string): Promise<DriverReport> => {
    const report = await invoke<DriverReport>('check_merge_drivers', { path })
    setState((s) => (samePath(s.path ?? '', path) ? { ...s, mergeDrivers: report } : s))
    return report
  }

  const refreshWorktrees = async (path: string) => {
    const wts = await invoke<Worktree[]>('read_worktrees_cmd', { path })
    setState((s) => ({ ...s, worktrees: wts }))
  }

  const refreshUpstream = async (path: string) => {
    const upstream = await invoke<UpstreamInfo>('read_upstream', { path })
    setState((s) => ({ ...s, upstream }))
  }

  const refreshStashes = async (path: string) => {
    const stashes = await invoke<StashEntry[]>('read_stashes', { path })
    setState((s) => ({ ...s, stashes }))
  }

  /**
   * 行尾规则装了没有（spec §7.3.0）。纯本地读。
   *
   * 失败时写回 `null` 而**不是**一个「不需要迁移」的值：两者在界面上都不挂横幅，
   * 但含义完全不同，把读失败当成「已经迁移过」会让这个仓库此后再也不提示。
   *
   * `samePath` 守卫同 `refreshSvnAudit`：读的过程中用户可能已经切到别的仓库，
   * 不比一下路径的话这次结果会写进另一个仓库的横幅里。
   */
  const refreshSvnEol = async (path: string) => {
    try {
      const eol = await svnEolStatus(path)
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnEol: eol } : s))
    } catch (err) {
      console.warn('读行尾规则状态失败：', err)
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnEol: null } : s))
    }
  }

  /**
   * 同步锁与上次未收尾的 journal（spec §5.1、§7.6）。
   *
   * `samePath` 守卫同上。失败**不写 `null`**：后端的 journal 坏了会以
   * `svn_journal_corrupt` 抛上来，写 `null` 就不挂横幅、也不报错——等于把它
   * 吞成「一切正常」。也不走顶部通用错误横幅：`repo-changed` 每次保存文件都会
   * 重读一次，那条错误关掉又会立刻弹回来。
   */
  const refreshSvnSyncState = async (path: string) => {
    try {
      const st = await svnSyncState(path)
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnSyncState: st } : s))
    } catch (err) {
      console.warn('读同步状态失败：', err)
      const unreadable: SyncStateUnreadable = { readError: toErrorMessage(err) }
      setState((s) =>
        samePath(s.path ?? '', path) ? { ...s, svnSyncState: unreadable } : s,
      )
    }
  }

  /**
   * 第二档本身：完整的 `nodeRevs` 校验（spec §6.6.3）。纯本地读，不碰观测值。
   *
   * 返回结论有没有写回：被更晚发起的一次顶替、或仓库已切走时是 `false`，调用方据此不记观测值
   * （见 `FullCheck`）。
   */
  const applySvnSync = async (path: string): Promise<boolean> => {
    // 只认最后发起的一次：焦点与 repo-changed 都会发起，先发起的那次看到的是更旧的工作副本，
    // 晚到时不能把 ExternallyAdvanced 盖回 InSync，失焦快照就是拿这个结论当前提的
    const token = refreshGate.begin('svnSync')
    let st: SyncStatus
    try {
      st = await svnSyncStatus(path)
    } catch (err) {
      if (refreshGate.isLatest(token, 'svnSync')) {
        latestVerdict.current = null
        // 基线分支丢了（第 70 条）：旧结论作废，横幅换成恢复入口。别的失败（网络、锁）
        // 什么都不动：既保留上一次的结论，也不清这个标记——基线确实丢了的时候，
        // 一次锁错误不该让恢复横幅闪掉（PR 115 review）
        if (toErrorCode(err) === 'svn_no_baseline') {
          // 提交前的提醒不能再按丢失之前的旧清单弹（6.6 第 2 轮 review 低危第 2 条）
          latestSync.current = null
          setState((s) => (samePath(s.path ?? '', path)
            ? { ...s, svnBaselineMissing: true, svnSync: null }
            : s))
        }
      }
      throw err
    }
    if (!refreshGate.isLatest(token, 'svnSync')) return false
    // `samePath` 守卫（第 71 条）：读的过程中用户可能已经切到别的仓库，旧仓库的结论
    // 写过去就是一条错的失同步横幅。`latestVerdict` 必须**同步**写（见 `openedPath`），
    // 所以按 ref 核对；state 照 `refreshSvnSyncState` 的写法在 updater 里再核对一次
    if (!samePath(openedPath.current ?? '', path)) return false
    latestVerdict.current = st.verdict.kind
    latestSync.current = st
    setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnSync: st } : s))
    return true
  }

  /** 给一次校验记数，从它读第一档的输入开始，到结论写回为止。 */
  const trackSvnCheck = async <T>(run: () => Promise<T>): Promise<T> => {
    svnChecksRunning.current += 1
    try {
      return await run()
    } finally {
      svnChecksRunning.current -= 1
    }
  }

  /** 立刻跑一轮检测（`runSvnSyncCheck`），从读戳到结论写回都记在 `svnChecksRunning` 里。 */
  const runSvnCheckNow = (path: string, force: boolean) =>
    trackSvnCheck(() =>
      runSvnSyncCheck(lastWcDbStamps.current, force, () => svnWcDbStamps(path), () => applySvnSync(path)),
    )

  /**
   * 后台的失同步检测都从这里排队（`createSvnCheckQueue`）：同一个仓库任一时刻只有一轮在跑，
   * 在跑时来的合并成一轮补跑，`force` 取或。计数包在开跑的那一轮上：排队期间前一轮还在跑，
   * 计数本来就不是 0。唯一不排队的是提交前的 `freshSvnSync`，见那里。
   */
  const requestSvnCheck = (path: string, force: boolean) =>
    svnCheckQueue.request(path, force, (forced) => runSvnCheckNow(path, forced))

  /**
   * 第二档的**无条件**入口：重建基线、同步之后用。打开仓库走 `forceCheckAtHead`（同样无条件，另外
   * 记下 HEAD），读不出 HEAD 时才退回这里。
   *
   * 顺带把 wc.db 戳记下来当基准——不记的话打开仓库后第一次存盘或焦点会因为记录还是 `null`
   * 而再跑一次全量。先读戳再跑校验，而且只在校验写回后才记（`runSvnSyncCheck`）。
   */
  const refreshSvnSync = async (path: string): Promise<void> => {
    await requestSvnCheck(path, true)
  }

  /**
   * 文件变动时的检测：wc.db 戳与**上次观测值**比，变了才跑第二档（`runSvnSyncCheck`）。
   *
   * 外部更新进行中 wc.db 一直在变，每个事件都并发起一轮的话会叠起多个秒级的 `svn status`，
   * 还和 TortoiseSVN 抢 wc.db 的锁（6.11 review 第 2 条），所以走 `requestSvnCheck` 排队。
   *
   * 返回是否真的跑了第二档，只为让调用方能读懂日志；调用方不依赖它。
   */
  const checkSvnSyncIfChanged = async (path: string) => (await requestSvnCheck(path, false)) !== 'skipped'

  /**
   * HEAD 动了就跑第二档（分析 §6.11.3）。`headBehindBase` 是第二档的结论，而切分支不改 revision、
   * 不写 wc.db，第一档看不见：不补这一条的话，切到落后的分支（包括外部工具切的）横幅上
   * 没有「跟进」，切回来又留着一条过时的横幅。只在 HEAD 真的变了时跑，存盘不会走到第二档。
   *
   * 排队时是强制的一轮：合并进比戳的补跑也不会被吞掉（见 `createSvnCheckQueue`）。写回了才记 HEAD
   * （`recordAfterSuccess`）。
   *
   * 比的是「已经请求过、还没落定的 HEAD」，没有才比上次记下的（独立 review）：一轮排队或在跑时
   * HEAD 从 a 切到 b 又切回 a，只比记录的话切回 a 那次看到的还是 a、直接返回，等 b 那轮写回把
   * 记录改成 b，HEAD 明明是 a，横幅却停在按 b 算的结论上，直到下一次 HEAD 或 wc.db 变化。
   */
  const checkSvnSyncIfHeadMoved = async (path: string, head: HeadObservation) => {
    const pending = headRequested.current
    const base = pending ?? lastCheckedHead.current.value
    if (base && sameHeadForSync(base, head)) return false
    // 本应用叠在还没落定的那一轮上的提交（`noteOwnCommit`）：那一轮落定时记成它，或者替它补一轮
    const own = ownCommitOnPending.current
    if (pending && own?.on === pending && sameHeadForSync(own.head, head)) return false
    await forceCheckAtHead(path, head)
    return true
  }

  /**
   * 按 `head` 强制跑一轮第二档，写回了才记成「上次校验的 HEAD」。`checkSvnSyncIfHeadMoved` 判定 HEAD 变了
   * 之后、打开仓库时都走这里。
   *
   * 跑的过程中 `headRequested` 是它（见 `checkSvnSyncIfHeadMoved` 为什么要比还没落定的 HEAD）。
   */
  const forceCheckAtHead = async (path: string, head: HeadObservation) => {
    headRequested.current = head
    let recorded = false
    try {
      recorded = await recordAfterSuccess(
        lastCheckedHead.current,
        head,
        async () => (await requestSvnCheck(path, true)) === 'applied',
      )
    } finally {
      // 更晚的一次已经接手时不清：它落定前，再来的同一个 HEAD 不必重复请求
      if (headRequested.current === head) headRequested.current = null
      // 按对象认登记时的那一轮，不按 hash：同一个提交上的另一轮（比如游离在同一个提交上）不是它
      const own = ownCommitOnPending.current
      if (own?.on === head) {
        ownCommitOnPending.current = null
        if (recorded && !headRequested.current && lastCheckedHead.current.value === head) {
          // 按 head 写回了：叠在它上面的本应用提交结论相同，直接记
          lastCheckedHead.current.value = own.head
        } else if (!headRequested.current) {
          // 没写回（失败、被顶替）：那次提交的事件已经被 `checkSvnSyncIfHeadMoved` 吞掉，不会再来，
          // 替它补一轮（第二轮独立 review 第 1、2 条）。更晚的请求接手时不补：它对应更新的 HEAD
          void checkSvnSyncIfHeadMoved(path, own.head).catch(() => undefined)
        }
      }
    }
    return recorded
  }

  /**
   * 本应用的普通提交（不是 amend）**成功之后**立刻调：现读 HEAD，是叠在已校验 HEAD 上的单父提交，就直接
   * 记成新的「上次校验的 HEAD」，监听随后读到它时不再跑第二档（`isPlainCommitOn`，2026-11-11 提交慢排查 H1）。
   *
   * 为什么在提交之后、而不是之前登记（独立 review 第 1、2、5 条）：之前登记的话，第一个不同的 HEAD 就会被
   * 当成本应用的提交，提交期间外部快进到基线也会被认领，落后横幅不撤。提交之后现读：提交期间分支被挪走时
   * `git commit` 本身会失败（不会走到这里），成功时紧接着读到的 HEAD 就是这次提交。不用 React state 里的
   * HEAD，那份可能还没刷新。
   *
   * 监听（去抖 300 ms）多半晚于这里；早于这里时它已经为新 HEAD 排了一轮校验，这里看到的
   * `headRequested` 就是新 HEAD 本身，不做任何事，只是没省下。
   */
  const noteOwnCommit = async (path: string) => {
    if (!samePath(openedPath.current ?? '', path)) return
    const head = await refreshHeadInfo(path)
    // 读的过程中仓库可能重开或切走（第二轮独立 review 第 3 条）：记录是按打开的那个仓库算的
    if (!samePath(openedPath.current ?? '', path)) return
    const pending = headRequested.current
    const base = pending ?? lastCheckedHead.current.value
    if (!base || !isPlainCommitOn(base, head)) return
    if (pending) ownCommitOnPending.current = { on: pending, head }
    else lastCheckedHead.current.value = head
  }

  /**
   * 窗口拿到焦点、或切到这个标签时的检测（分析 §6.6）：wc.db 变了才跑第二档。
   *
   * 不能只靠 `checkSvnSyncIfChanged`：TortoiseSVN 只提交、不更新时没有文件事件
   * （`.svn` 被 watcher 过滤），只有 wc.db 变了。
   */
  const checkSvnSyncOnFocus = async (path: string) => (await requestSvnCheck(path, false)) !== 'skipped'

  /**
   * 提交前要用的结论（事故分析 6.6，独立 review 第 4 条）：TortoiseSVN 刚更新完、焦点检测还没写回时，
   * `state.svnSync` 还是旧的 InSync，提醒就漏了。先跑一次焦点检测：wc.db 没变只是几次 stat，变了才跑
   * 第二档并等它写回。再从同步写的 ref 读，不读闭包里的 state。检测失败时用已有的结论，不挡提交。
   *
   * **不排队**（独立 review）：排在后台那轮后面的话，那轮卡住（svn 进程不退出）提交就跟着卡住，
   * 不卡也要多等一整轮。自己立刻跑一轮，结论一定晚于这次请求；后台那轮晚到时被 `refreshGate`
   * 顶替，不会盖掉它。代价是提交那一刻可能与后台那轮并发一次。
   */
  const freshSvnSync = async (path: string): Promise<SyncStatus | null> => {
    await runSvnCheckNow(path, false).catch(() => undefined)
    return samePath(openedPath.current ?? '', path) ? latestSync.current : null
  }

  /**
   * 窗口失焦、或切到别的标签时记一份快照。
   *
   * 写不成不打扰用户：快照只是判定线索，没有它时跟进走「拿不准」的保守分支。
   */
  const markSvnFollowOnBlur = async (path: string) => {
    try {
      await markOnBlur(
        latestVerdict.current,
        svnChecksRunning.current,
        lastWcDbStamps.current,
        () => svnWcDbStamps(path),
        () => svnFollowMark(path),
      )
    } catch (err) {
      console.warn('失焦快照没写成：', err)
    }
  }

  /** 落后基线的分支。`samePath` 守卫同 `refreshSvnEol`。 */
  const refreshSvnBranchesBehind = async (path: string) => {
    const behind = await svnBranchesBehind(path)
    setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnBranchesBehind: behind } : s))
  }

  /** 读缓存里的 SVN 历史并接进图。不走网络。 */
  const refreshSvnLog = async (path: string) => {
    const page = await svnReadLog(path)
    applySvnLog(path, page)
  }

  /**
   * 把一页 SVN 历史写回并重接提交图。
   *
   * svnRevs 一变就要重接一次：后台拉取是异步的，先画出来的图里
   * svn 节点还没到位，工作分支那时还挂在 baseline commit 上。
   *
   * `samePath` 守卫（第 71 条）：拉取走网络、可能好几秒，期间切到别的仓库的话，
   * 旧仓库的 SVN 节点会被接进新仓库的图。
   */
  const applySvnLog = (path: string, page: SvnLogPage) => {
    setState((s) => (samePath(s.path ?? '', path)
      ? {
          ...s,
          svnRevs: page.revs,
          commits: merge(s, s.gitCommits, page.revs),
          svnLogComplete: page.complete,
        }
      : s))
  }

  /**
   * 走网络拉 SVN 历史。**失败只写 console 不上报**——顶部错误横幅是留给 git
   * 操作的，被这条锦上添花的信息占住的话，真正的失败反而看不见
   * （与 `refreshPullRequests` 同一条理由）。
   */
  const fetchSvnLog = async (path: string, older = false) => {
    try {
      applySvnLog(path, await svnFetchLog(path, older, SVN_LOG_PAGE))
    } catch (err) {
      console.warn('拉取 SVN 历史失败，图上只显示缓存里已有的部分：', err)
    }
  }

  /**
   * 服务器侧清单核对。走网络。
   *
   * `keepOnError` 分开两种调用场景，它们对失败的正确反应**相反**：
   *
   * - 打开仓库 / 重建之后（`false`）：必须**写回 null**。这次的结果是权威的，
   *   失败时留着上一个仓库的结论，会把 A 仓库的漏收挂到 B 仓库头上。
   * - 后台自动重跑（`true`）：必须**保留**上一次的结论。一次网络抖动就把一条
   *   已经成立的漏收警告抹掉，是把真警报降级成沉默——而这条检查存在的全部理由
   *   就是那个沉默很危险。
   */
  const refreshSvnAudit = async (path: string, keepOnError = false) => {
    lastAuditAt.current = Date.now()
    try {
      const audit = await svnServerAudit(path)
      // 核对期间用户可能已经切到别的仓库了。不比一下路径的话，这次的结果会写进
      // 另一个完全无关的仓库的横幅里——后台定时重跑之后这个窗口大了很多。
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnAudit: audit } : s))
    } catch (err) {
      console.warn('服务器侧清单核对失败：', err)
      if (keepOnError) return
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnAudit: null } : s))
    }
  }

  /**
   * 后台自动重跑的入口：过了闸才真的发请求（`auditIsDue`）。
   *
   * 这条检查要抓的是用户**不知情**的漏收，所以不能只靠手动入口——不知道出事的人
   * 不会去点那颗按钮（2026-09-19 用户决定）。
   */
  const auditSvnIfDue = async (path: string) => {
    if (!auditIsDue(Date.now(), lastAuditAt.current, auditInFlight.current)) return
    auditInFlight.current = true
    try {
      await refreshSvnAudit(path, true)
    } finally {
      auditInFlight.current = false
    }
  }

  /**
   * 重新读取被钉死的 external（spec §6.5.3 的常驻说明）。
   *
   * 失败只写 console 不上报，理由与 `refreshPullRequests` 同：顶部错误横幅是留给
   * git 操作的。**必须写回空数组**——上一个仓库处于 pinned 模式时留下的清单，
   * 会挂在下一个完全无关的仓库上。
   */
  const refreshPinned = async (path: string) => {
    try {
      const pinned = await svnPinnedExternals(path)
      // `samePath` 守卫：`svn propget -R` 是秒级、打开仓库时不 await，期间可能已切走（第 71 条）
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, pinned } : s))
    } catch (err) {
      console.warn('读取 pinned external 失败，不显示 pinned 模式说明：', err)
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, pinned: [] } : s))
    }
  }

  /**
   * 重新读取 open PR 列表。
   *
   * **只在打开仓库和用户主动 fetch/pull/push 之后调用**，绝不能挂进
   * `repo-changed` 那条文件监听通道：那条通道一次编辑就触发一次，而这里要走网络。
   *
   * 失败只写 console 不上报：顶部错误横幅是留给 git 操作的，被这条锦上添花的信息
   * 占住的话，真正的失败反而看不见。后端在 gh 缺失/未登录/非 GitHub 仓库时已经
   * 返回空列表，所以走到 catch 的只剩「gh 输出解析不了」这种本项目自己的 bug。
   */
  const refreshPullRequests = async (path: string) => {
    try {
      const prs = await invoke<PullRequest[]>('list_pull_requests', { path })
      // `samePath` 守卫：gh 走网络一两秒，期间可能已切走（第 71 条）
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, pullRequests: prs } : s))
    } catch (err) {
      console.warn('读取 PR 列表失败，分支旁不显示编号徽章：', err)
    }
  }

  /**
   * @param joinSince 给了就允许复用正在跑的那一轮：同一个 scope、且那一轮在第 `joinSince` 次写操作刷新
   *   开始之后才开读（`writeSeq`）。按选中读的 effect 传 0（任何一轮都行，它只要数据）；写操作之后的刷新
   *   传自己开始时的序号（写之前就在读的那轮读到的是旧数据，不能复用）。不给就照旧排补跑
   *   （文件监听：在跑的那轮可能早于这次改动）。没有它的话，Stage 让选中的文件换到 Staged 时，
   *   写之后的刷新和选中跟着变的 effect 会把同一个文件连读两次（第二轮独立 review 第 7 条）
   */
  const refreshDiff = async (path: string, scope: string, joinSince?: number) => {
    // 写回前再核对：只认当前视图要的那一份，切走之后才回来的旧 scope 结果丢掉
    lastDiffPath.current = path
    // 读之前开关又变了（比如连按两下「增加上下文」）的那一份也丢掉：变了的那一下已经排了一轮新的
    const stillWanted = (optionsKey: string) => {
      if (lastDiffPath.current !== path) return false
      if (currentDiffOptions().key !== optionsKey) return false
      const wanted = wantedDiffScope.current
      return wanted === undefined || wanted === scope
    }
    // 按仓库和 scope 分开单飞（同 `refreshStatus`）：别的 scope 的读取不用排队。
    // 同一个 scope 各轮串行、按开读先后到达，写回不必再核对代次。
    // 「本地改动」的单文件 scope 例外，全部排在同一队里：方向键一路按下去，每个文件都会来一次，
    // 补跑只做最后一个，中间跳过的不读（6.13 方案 C）。各轮仍串行，写回照旧只认当前要的那一份
    const flight = isChangesFileScope(scope) ? `diff:${path}:changes-file` : `diff:${path}:${scope}`
    const running = diffRunning.current.get(flight)
    if (
      joinSince !== undefined && running?.scope === scope && running.optionsKey === currentDiffOptions().key
      && running.seq >= joinSince
    ) {
      // 复用的调用方只等它读完，成败由发起那一轮的调用方报
      return running.done
    }
    await refreshFlights.run(flight, async () => {
      let finish!: () => void
      // 开关在真正开读的这一刻取：排在补跑里的这一轮可能是开关变了之前登记的
      const { options, key: optionsKey } = currentDiffOptions()
      const entry = { scope, optionsKey, seq: writeSeq.current, done: new Promise<void>((r) => { finish = r }) }
      diffRunning.current.set(flight, entry)
      try {
        let diff: DiffData
        try {
          diff = await invoke<DiffData>('read_diff', { path, scope, options })
        } catch (err) {
          // 读失败也要把这个 scope 记成「读过了」，不然界面会一直说「正在读取」；错误照旧抛给调用方。
          // 带上原因：光一个空列表，界面上就成了「这个提交没有改动」（第 72 条）
          // 用户已经离开的那份读失败了就安静地丢掉：照常抛的话，界面显示的是 B 的 diff，
          // 顶部错误条却报 A 的「bad object」（PR 113 review）
          if (!stillWanted(optionsKey)) return
          setState((s) => ({ ...s, diff: { files: [], error: toErrorMessage(err) }, diffScope: scope }))
          throw err
        }
        if (stillWanted(optionsKey)) setState((s) => ({ ...s, diff, diffScope: scope }))
      } finally {
        if (diffRunning.current.get(flight) === entry) diffRunning.current.delete(flight)
        finish()
      }
    })
  }

  /**
   * 写操作（Stage / Unstage / 丢弃 / 提交 / DU-UD 二选一）之后刷新「本地改动」。
   *
   * 按**刷新后**的状态算该读哪个区（第 65 条）：选中的文件可能刚换了区，按点击那一刻的区去读，
   * 读回来的整份 diff 只会被丢掉。`selection` 是取值函数，调用方传最新的选中。
   *
   * @param touched 只改了 index 的操作（Stage / Unstage）传刚操作的路径，状态只重读这几个路径。
   *   会动工作区的操作（丢弃、提交、DU/UD 二选一）不传，照常全量读
   */
  const refreshChanges = async (path: string, selection: () => FileSelection, touched?: string[]) => {
    // 这次写操作之后才开读的 diff 可以复用，之前的不行（`refreshDiff` 的 `joinSince`）
    const since = ++writeSeq.current
    const status = await refreshStatus(path, touched)
    const scope = changesDiffScope(selection(), status)
    if (scope) await refreshDiff(path, scope, since)
  }

  /**
   * 本应用自己写了 index（Stage / Unstage）之后调，补上原来靠监听那一轮顺带做的事。
   *
   * 自己写的 index 不再触发 `repo-changed`（`commands::watch::OwnIndexWrite`，6.13 方案 B），于是：
   * - merge driver 要重查：检查的指纹覆盖 index 里的路径和暂存的 `.gitattributes`（独立 review 第 2 条）
   * - 同一个仓库开在别的标签里时，那些标签收不到任何事件，要通知它们全量刷新（第 6 条）
   */
  const afterIndexWrite = (path: string) => {
    void refreshMergeDrivers(path).catch((err) => console.warn('merge driver 检查失败：', err))
    for (const notify of indexWriteSubscribers) notify(path, storeIdentity)
  }

  // 别的标签写了同一个仓库的 index：全量刷新状态和当前视图的 diff。
  // 依赖为空：处理函数只读 ref，刷新函数也只碰 ref 和 setState，闭包住第一次渲染的那份不会读到旧值
  useEffect(() => {
    const onIndexWrite = (path: string, origin: object) => {
      if (origin === storeIdentity) return
      const mine = lastStatusPath.current
      if (!mine || !samePath(mine, path)) return
      void refreshStatus(mine).catch(() => undefined)
      // merge driver 的检查也看 index（暂存的 .gitattributes），同样要重查（第二轮独立 review 第 2 条）
      void refreshMergeDrivers(mine).catch(() => undefined)
      const scope = wantedDiffScope.current
      if (scope) void refreshDiff(mine, scope).catch(() => undefined)
    }
    indexWriteSubscribers.add(onIndexWrite)
    return () => { indexWriteSubscribers.delete(onIndexWrite) }
  }, [])

  const selectCommit = (id: string) => {
    setState((s) => ({ ...s, selectedId: id }))
  }

  const clearError = useCallback(
    () => setState((s) => ({ ...s, error: null, errorCode: null })),
    [],
  )

  /** 把一次失败呈现到顶部错误横幅。 */
  const reportError = useCallback(
    (err: unknown) =>
      setState((s) => ({ ...s, error: toErrorMessage(err), errorCode: toErrorCode(err) })),
    [],
  )

  return {
    state,
    openRepo,
    refreshHead,
    refreshHeadInfo,
    refreshGraph,
    refreshStatus,
    refreshWorktrees,
    refreshMergeDrivers,
    refreshDiff,
    refreshUpstream,
    refreshStashes,
    refreshPullRequests,
    refreshSvnSync,
    refreshSvnEol,
    refreshSvnSyncState,
    refreshSvnBranchesBehind,
    checkSvnSyncIfChanged,
    checkSvnSyncIfHeadMoved,
    noteOwnCommit,
    checkSvnSyncOnFocus,
    freshSvnSync,
    markSvnFollowOnBlur,
    refreshSvnLog,
    fetchSvnLog,
    refreshSvnAudit,
    auditSvnIfDue,
    selectCommit,
    clearError,
    reportError,
    wantedDiffScope,
    refreshChanges,
    afterIndexWrite,
  }
}

/** `useRepoEffects` 需要的刷新函数集合。 */
export interface RepoRefreshers {
  refreshHeadInfo: (path: string) => Promise<HeadObservation>
  refreshGraph: (path: string) => Promise<void>
  refreshStatus: (path: string) => Promise<StatusData>
  refreshWorktrees: (path: string) => Promise<void>
  /** `.gitattributes` 或 git 配置变了要重查；后端按指纹缓存，没变时不重扫。 */
  refreshMergeDrivers: (path: string) => Promise<DriverReport>
  refreshDiff: (path: string, scope: string, joinSince?: number) => Promise<void>
  refreshUpstream: (path: string) => Promise<void>
  refreshStashes: (path: string) => Promise<void>
  /** §6.6.3 第二档。只在第一档报告「有人动过」之后才调。 */
  /** §6.6.3 的第一档 + 条件触发第二档。挂在高频的 repo-changed 上。 */
  checkSvnSyncIfChanged: (path: string) => Promise<boolean>
  /** §6.11.3：HEAD 变了才跑第二档。挂在 repo-changed 读到的新 HEAD 上。 */
  checkSvnSyncIfHeadMoved: (path: string, head: HeadObservation) => Promise<boolean>
  /** §6.6 的焦点检测：wc.db 变了才跑第二档。挂窗口焦点与切标签。 */
  checkSvnSyncOnFocus: (path: string) => Promise<boolean>
  /** 失焦快照，自带是否该写的判断，不抛。挂窗口失焦与切标签。 */
  markSvnFollowOnBlur: (path: string) => Promise<void>
  refreshSvnSyncState: (path: string) => Promise<void>
  refreshSvnBranchesBehind: (path: string) => Promise<void>
  /** 服务器侧核对的**后台**入口，自带频率闸。挂定时器与窗口焦点。 */
  auditSvnIfDue: (path: string) => Promise<void>
  /** `useRepoStore` 的 `wantedDiffScope`：这里每次渲染写入当前视图的 scope。 */
  wantedDiffScope: MutableRefObject<string | null | undefined>
  /** 用户操作引起的读取失败交给顶部错误条。 */
  reportError: (err: unknown) => void
}

/**
 * @param changesScope 「本地改动」视图右侧 diff 的 scope（`changesDiffScope`）：选中文件在 Unstaged 是
 *   `unstaged:<路径>`（暂存区对比工作区），在 Staged 是 `staged:<路径>`（HEAD 对比暂存区），见第 65 条；
 *   没有文件可显示时为 null
 * @param active 这个标签是不是当前显示的那个。所有标签都挂载着，焦点事件每个标签都收得到
 */
export function useRepoEffects(
  state: RepoState,
  refreshers: RepoRefreshers,
  viewMode: string,
  changesScope: string | null,
  active: boolean,
) {
  const { refreshHeadInfo, refreshGraph, refreshStatus, refreshWorktrees, refreshMergeDrivers, refreshDiff, refreshUpstream, refreshStashes, checkSvnSyncIfChanged, checkSvnSyncIfHeadMoved, checkSvnSyncOnFocus, markSvnFollowOnBlur, refreshSvnSyncState, refreshSvnBranchesBehind, auditSvnIfDue, reportError } = refreshers
  // svn 节点有两版本地基线时读两版之差（`range:`）；不在图上的选中（stash）照旧按 oid 读
  const diffScope = viewMode === 'changes'
    ? changesScope
    : selectionDiffScope(state.commits, state.selectedId)
  // 在渲染里写而不是在 effect 里写：下面那个按选中刷新 diff 的 effect 发请求之前，它必须已经是新值
  refreshers.wantedDiffScope.current = diffScope
  // 上下文行数 / 忽略空白一变就重读；其余开关只是换个画法，键不变，不发请求（`backendDiffOptions`）
  const diffOptions = useDiffOptionsKey()
  /** 这个标签上一次因为下面这个 effect 读的是哪一份、按什么开关读的。 */
  const lastDiffRead = useRef<{ target: string, options: string } | null>(null)

  useEffect(() => {
    if (!state.path || !diffScope) return
    // 开关是全局的，一变所有标签都会走到这里。没显示的标签先不读，切回来（`active` 变 true）时再补：
    // 否则开一次「整个文件」，后台每个标签都把自己那个提交整份全文读一遍（第二轮独立 review 第 1 条）
    const target = `${state.path}\n${diffScope}\n${state.selectedId ?? ''}`
    const last = lastDiffRead.current
    if (last?.target === target && (last.options === diffOptions || !active)) return
    lastDiffRead.current = { target, options: diffOptions }
    // 不 catch 的话失败是一个未处理的 rejection，顶部错误条什么都不显示（第 72 条）。
    // 与文件监听那条不同，这里是用户点了才读的，失败要让他看见
    // 只要这一份数据：已经在读同一个 scope 就复用那一轮（Stage 之后写操作的刷新多半刚发出它）
    refreshDiff(state.path, diffScope, 0).catch(reportError)
  }, [state.selectedId, state.path, diffScope, diffOptions, active])

  useEffect(() => {
    if (!state.path) return
    let unlisten: (() => void) | undefined
    // 依赖里**没有** diffScope：「本地改动」的 scope 随选中的文件变（6.13 方案 C），方向键每按一下就变，
    // 跟着重新注册的话注销与重新注册之间那一次 IPC 往返里的事件会丢（独立 review 第 4 条）。处理函数按
    // 事件到达时的 `wantedDiffScope` 读，不需要它。`vcs` 在依赖里：处理函数按它决定要不要跑 SVN 那几项。
    // 换仓库时仍会重新注册。`listen` 是一次 IPC 往返，清理时它可能还没回来：回来之后立刻注销，
    // 否则每换一次就多挂一个监听器（同 `useExternalToolExit`）
    let disposed = false
    // 尾随复查（事故分析 6.11）：wc.db 的最后一笔写入没有文件事件，见 `SVN_TRAILING_RECHECK_MS`
    let trailing: number | undefined
    listen<string>('repo-changed', (event) => {
      const path = state.path!
      // 事件是全局广播的，每个标签只认自己那个仓库的变更
      if (!samePath(event.payload, path)) return
      // 刷新期间仓库可能正处于中间态（比如 checkout 写到一半），失败就等下一次事件
      const ignore = () => undefined
      // 外部 checkout 同样会改 HEAD 和 worktree 归属，两者都要跟着刷新
      void refreshHeadInfo(path)
        .then((head) => {
          if (state.capabilities.vcs === 'svn') return checkSvnSyncIfHeadMoved(path, head)
        })
        .catch(ignore)
      void refreshGraph(path).catch(ignore)
      void refreshStatus(path).catch(ignore)
      void refreshWorktrees(path).catch(ignore)
      // 外部 push/fetch 会改变领先落后数，命令行 stash 也要反映到面板上
      void refreshUpstream(path).catch(ignore)
      void refreshStashes(path).catch(ignore)
      void refreshMergeDrivers(path).catch(ignore)
      // 按事件到达时当前视图要的那份读，不按注册时的 diffScope：Stage 之后选中的文件
      // 可能已经换了区，旧区那一份读回来只会被丢掉
      const scope = refreshers.wantedDiffScope.current
      if (scope) void refreshDiff(path, scope).catch(ignore)
      // §6.6.3 的**分层**：高频通道上只跑第一档（比 wc.db 戳，戳没变时后端只有几次 `stat`），
      // 变了才跑第二档。
      //
      // 不分层的话，每一次文件保存都会触发一遍白名单子树的 `svn status -v`
      // ——那是本地读，但在几万文件的工作副本上仍然是可感知的开销。第一档本身也不能起
      // svn 进程：原来读各工作副本根的 revision，每次都要重新枚举工作副本，公司机器上一轮
      // 69 个进程、约 10 秒（计划 2026-11-08 Task 1）。
      //
      // 比对用的「上次观测值」存在 **ref** 里（见 `useRepoStore` 的
      // `lastWcDbStamps`）：这个监听器的 `useEffect` 依赖是 `[state.path, state.capabilities.vcs]`，
      // 把它放进 state 的话闭包会永远读到注册那一刻的值，比较恒为不等，
      // 第二档还是每次都跑——分层白做。
      if (state.capabilities.vcs === 'svn') {
        void checkSvnSyncIfChanged(path).catch(ignore)
        window.clearTimeout(trailing)
        trailing = window.setTimeout(() => {
          if (!disposed) void checkSvnSyncIfChanged(path).catch(ignore)
        }, SVN_TRAILING_RECHECK_MS)
        // 同步状态也要跟着刷（2026-09-25 独立 review）。只在打开仓库时读一次的话：
        // 另一个进程开始或结束的同步永远不会出现 / 永远不会消失；更糟的是，
        // 打开那一刻若恰好读到 `locked: true`，`otherBannersAreSuppressed` 会把
        // 失同步 / 行尾 / 漏收横幅**压住整个标签页的生命周期**，除非关掉重开。
        //
        // 它是纯本地读（一次 stat + 一个小文件），挂在高频通道上不需要分层。
        void refreshSvnSyncState(path).catch(ignore)
        // 跟进、切分支、外部工具动了分支都走这里。后端走 libgit2，不起进程
        void refreshSvnBranchesBehind(path).catch(ignore)
      }
    }).then((fn) => {
      if (disposed) fn()
      else unlisten = fn
    })
    return () => {
      disposed = true
      window.clearTimeout(trailing)
      unlisten?.()
    }
  }, [state.path, state.capabilities.vcs])

  /**
   * 锁被持有期间轮询同步状态，直到它松开（spec §5.1）。
   *
   * 锁的释放不一定产生文件事件：持锁进程崩溃、或只是关掉句柄时，锁文件原样
   * 留着，watcher 什么都收不到。不轮询的话黄条会一直挂着，还一直压着其余横幅，
   * 直到工作区碰巧有别的变动——用户不该被要求「关掉标签页再打开」。
   *
   * 读到 `locked: false` 后 `lockPolling` 变假，effect 清理掉定时器，轮询自行停止。
   * 没有锁的时候（绝大多数时间）不装任何定时器。
   */
  const lockPolling =
    state.capabilities.vcs === 'svn' && needsLockPolling(state.svnSyncState)
  useEffect(() => {
    if (!state.path || !lockPolling) return
    const path = state.path
    const timer = window.setInterval(() => {
      void refreshSvnSyncState(path).catch(() => undefined)
    }, SYNC_LOCK_POLL_MS)
    return () => window.clearInterval(timer)
    // 依赖里不放 `refreshSvnSyncState`，理由同下面审计定时器那段注释。
  }, [state.path, lockPolling])

  /**
   * 服务器侧漏收核对的**自动**触发（2026-09-19）。
   *
   * # 为什么不挂在 repo-changed 上
   *
   * 那条通道一次文件保存就触发一次，而这里要走网络（`svn ls -R`）。
   * `refreshPullRequests` 的注释里记的是同一条理由。
   *
   * # 为什么不能只留手动入口
   *
   * `--set-depth=exclude` 造成的漏收，SVN 侧四道检查全是绿的、磁盘上也看不见。
   * 用户不知情，就不会去点「检查一下」。所以它必须自己跑起来。
   *
   * # 两个触发点
   *
   * - **定时器**：兜底，覆盖「应用一直开着、后台脚本动了工作副本」。
   * - **窗口获得焦点**：用户刚从 TortoiseSVN / 终端切回来——最可能刚出事的时刻。
   *   它会被 `SVN_AUDIT_MIN_GAP_MS` 那道闸限频，来回切窗口打不出高频请求。
   *
   * 页面不可见时跳过：最小化着的窗口每五分钟发一次网络请求没有意义。这不是
   * 安全关键——闸本身已经把最坏情况钉住了，这里只是省掉明显的浪费。
   */
  useEffect(() => {
    if (!state.path || state.capabilities.vcs !== 'svn') return
    const path = state.path
    const tick = () => {
      if (document.hidden) return
      void auditSvnIfDue(path).catch(() => undefined)
    }
    const timer = window.setInterval(tick, SVN_AUDIT_POLL_MS)
    window.addEventListener('focus', tick)
    return () => {
      window.clearInterval(timer)
      window.removeEventListener('focus', tick)
    }
    // ⚠️ 依赖里**不放** `auditSvnIfDue`：它每次渲染都是新函数，放进去会让这个
    // effect 每渲染一次就重装一次定时器，五分钟的计时永远归零、一次都不会触发。
    // 本文件里其它几个 effect 也是同一个写法。
  }, [state.path, state.capabilities.vcs])

  /**
   * 外部提交的检测与失焦快照（分析 §6.6，日志 W26）。
   *
   * 用户的操作顺序是「切到 TortoiseSVN 提交 → 切回来」：失焦那一刻记下暂存与 HEAD，
   * 它们一定早于这次提交；拿到焦点时比 wc.db，发现提交。
   *
   * 只有当前标签挂监听：别的仓库的暂存不是用户此刻在看的，检测也等切过去再做（下一个 effect）。
   * 依赖里不放这两个函数，理由同上面的审计定时器。
   */
  useEffect(() => {
    if (!state.path || state.capabilities.vcs !== 'svn' || !active) return
    const path = state.path
    const onFocus = () => {
      void checkSvnSyncOnFocus(path).catch((err) => console.warn('焦点检测失败：', err))
    }
    const onBlur = () => {
      void markSvnFollowOnBlur(path)
    }
    window.addEventListener('focus', onFocus)
    window.addEventListener('blur', onBlur)
    return () => {
      window.removeEventListener('focus', onFocus)
      window.removeEventListener('blur', onBlur)
    }
  }, [state.path, state.capabilities.vcs, active])

  /**
   * 切标签等于这个仓库的失焦 / 拿到焦点：标签藏着时外部提交没人检测，切走之后的窗口失焦又
   * 轮不到它。只响应 `active` 真的变了，第一次渲染打开仓库自己会校验。
   */
  const wasActive = useRef(active)
  useEffect(() => {
    if (wasActive.current === active) return
    wasActive.current = active
    if (!state.path || state.capabilities.vcs !== 'svn') return
    if (active) {
      void checkSvnSyncOnFocus(state.path).catch((err) => console.warn('焦点检测失败：', err))
    } else {
      void markSvnFollowOnBlur(state.path)
    }
  }, [active])
}
