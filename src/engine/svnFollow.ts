/**
 * 跟进外部提交（分析 §6.5）与手动跟进（计划 Task 13）的类型与文案。
 *
 * 纯函数，无 React / Tauri 依赖（`AGENTS.md` §7.2）。跟进完成时说什么在 {@link describeFollowReport}，
 * 没完成时说什么在 {@link describeFollowOutcome}。
 *
 * ⚠️ 横幅与对话框都按**纯文本**渲染：不要写 `**`。
 */
import { externalChangeLine, type ExternalChange, type SyncVerdict } from './svnSync'
import type { DiffTarget } from './externalTools'
import { formatDuration } from './svnInit'
import {
  describeFollowDrift,
  stageLostDetail,
  type NoticeTone,
  type OutcomeNotice,
  type SyncActionId,
  type SyncConfirmModel,
  type SyncInOutcome,
} from './svnSyncIn'

/** 后端 `svn_leak::LeakVerdict`。 */
export type LeakVerdict = 'present' | 'unknown'

/** 后端 `svn_leak::HunkLeak`。整文件判定时 `oldStart` 与 `lines` 都是 0。 */
export interface HunkLeak {
  oldStart: number
  lines: number
  verdict: LeakVerdict
  /** 第一行改动（`+…` / `-…`），给人辨认用；整文件判定或内容不是 UTF-8 时没有这个字段。 */
  preview?: string
}

/** 后端 `svn_leak::Leak`。 */
export interface Leak {
  branch: string
  commit: string
  subject: string
  path: string
  hunks: HunkLeak[]
}

/** 后端 `svn_follow_index::StageDecision`。 */
export type StageDecision = 'takeNew' | 'keepStaged' | 'lostToNew' | { merged: string }

/** 后端 `svn_follow_index::ComposeReport`。 */
export interface ComposeReport {
  decisions: Record<string, StageDecision>
  uncertain: string[]
  /** 暂存没能保留，而且其中有改动不在 SVN 新版本里。 */
  lost: string[]
  discarded: string[]
  /** 暂存没能保留，但它的每处改动 SVN 新版本里都有了（方案 C）。旧后端没有。 */
  superseded?: string[]
  /** `lost` 里一部分改动已在 SVN 上的：路径 → 计数。旧后端没有。 */
  partial?: Record<string, { present: number; total: number }>
}

/** 后端 `svn_follow::OverlapFile`：分支上的提交改过、SVN 侧也有新内容的文件（分析 §6.12 条件 11）。 */
export interface OverlapFile {
  path: string
  /** 改过它、重放后仍保留的提交：`短 hash 提交说明`。 */
  commits: string[]
  /** 重放出来的分支版本与磁盘逐字节相同。 */
  matchesDisk: boolean
}

/** 后端 `git_read::LineStat`：同 `git diff --numstat` 的一行，二进制时行数为 0。 */
export interface LineStat {
  added: number
  removed: number
  binary: boolean
}

/**
 * 「分支比 SVN 少 7 行」：对不上的文件差在哪（计划 2026-11-16 的 B）。说不出来时 `null`。
 *
 * `svnRemoved`：SVN 上已删、分支把它带了回来。这时多出来的是整个文件，只写「多 120 行」看不出来
 * （独立 review nit 5）。
 */
function driftLinesText(l: LineStat | undefined, svnRemoved: boolean): string | null {
  if (svnRemoved) {
    const n = l && !l.binary && l.added > 0 ? `（${l.added} 行）` : ''
    return `SVN 上已删除，分支里还有这个文件${n}`
  }
  if (!l) return null
  if (l.binary) return '二进制文件，内容不同'
  const parts = [l.added > 0 ? `多 ${l.added} 行` : '', l.removed > 0 ? `少 ${l.removed} 行` : '']
  const said = parts.filter((p) => p !== '').join('、')
  return said ? `分支比 SVN ${said}` : null
}

/** 后端 `svn_follow::DriftRange`：两个提交的 OID。 */
export interface DriftRange {
  from: string
  to: string
}

/**
 * 「对比」交给外部工具的两侧：左边（from）是 SVN 的版本，右边（to）是分支的版本。报告框的按钮提示
 * 照这个方向写；方向一反，用户会把分支的版本当成 SVN 的去提交（独立 review 建议 1）。
 */
export function driftDiffTarget(range: DriftRange, file: string): DiffTarget {
  return { kind: 'range', from: range.from, to: range.to, file, oldFile: null }
}

/** 后端 `svn_follow::HeldBack`：部分自动跟进留在旧基线上的一个文件。 */
export interface HeldBack {
  path: string
  /** 整次跟进时拦下它的那条原因。 */
  reason: string
}

/** 后端 `svn_follow::FollowReport`。 */
export interface FollowReport {
  newFloor: number
  /** 这次录的基线经过了服务器体检。断网时是 false：基线照样发布了。 */
  verified: boolean
  changes: ExternalChange[]
  removed: string[]
  dropped: string[]
  /** (原 commit, 换成 SVN 版本的路径)：我自己提交到 SVN 的（F2 ①）。 */
  rewritten: [string, string[]][]
  /** (原 commit, 换成 SVN 版本的路径)：内容与更新的某个基线逐字节相同，是快照进来的 SVN 内容（6.10）。 */
  snapshot: [string, string[]][]
  /** 重放后与磁盘对不上的文件：多半是重放干净却合错了（PR 119 遗留）。 */
  drift: string[]
  /**
   * `drift` 里每个文件分支相对磁盘（= SVN 版本）的增删行数（计划 2026-11-16 的 B）。`added` 是分支
   * 多出来的行，`removed` 是分支少了的行。旧后端、或者后端算不出来时没有。
   */
  driftLines?: Record<string, LineStat>
  /**
   * `drift` 比的两个提交：`from` 是新基线（里面就是磁盘上的 SVN 版本），`to` 是重放后的分支 tip。
   * 报告里每个对不上的文件旁边的「对比」用外部工具打开这两侧。旧后端、没有 drift 时没有。
   */
  driftRange?: DriftRange
  /**
   * `drift` 里能用「放回 Local-Only 补丁」写回磁盘的（使用报告 9.1）：分支比磁盘多出来的只有
   * Local-Only 改动，磁盘就是 SVN 的版本。旧后端没有。
   */
  restorable?: string[]
  /**
   * 跟进完成后自动放回的结果（计划 2026-11-12-auto-restore-local-only）。`restorable` 里没放回的
   * 都在 `skipped` 里写着原因。`restorable` 为空、或者旧后端时没有。
   */
  restore?: RestoreOutcome | null
  /** 条件 11 的重叠文件，逐个标出是否与磁盘一致。旧后端没有。 */
  overlap?: OverlapFile[]
  /**
   * 部分自动跟进时留在旧基线上的文件与原因（计划 2026-11-13-partial-auto-follow）。它们 SVN 侧的
   * 新内容这次没跟，横幅会继续列出。整次跟进、旧后端时没有或为空。
   */
  heldBack?: HeldBack[]
  stage: ComposeReport
  movedBranches: string[]
  behindBranches: [string, string][]
  leaks: Leak[]
  /**
   * 在 TortoiseSVN 提交框里看过 jkk 的警告、点 Proceed anyway 提交了的泄漏（设计
   * 2026-11-16-tsvn-acknowledged-leaks）。只留档，不拦自动跟进。旧后端没有。
   */
  acknowledgedLeaks?: Leak[]
  /** 钉着换入之前的原始索引与暂存 blob 的 ref。 */
  pinnedRef: string | null
  /** 判定依据（C1h）：钩子健康时按本机事件，否则按作者 + 失焦快照。 */
  evidence: 'hooks' | 'heuristic'
  /** 读收件箱的时刻。前端不用，随 journal 走。 */
  evidenceCutoffMs: number | null
  /**
   * 这次跟进命令从发出到返回的毫秒数。**后端不发**，流程在命令返回后补上（`svnSyncFlow`），
   * 报告页脚据此写「跟进用时」。不含之后的界面刷新；崩溃后续上的那次没有。
   */
  elapsedMs?: number
}

/** 后端 `svn_follow::ConflictDetail`：冲突框里一个冲突文件的上下文（事故分析 6.5）。 */
export interface ConflictDetail {
  path: string
  /** SVN 上最后一次改它的 revision 与作者。本次外部变更里没有它时为 null。 */
  svnRev: number | null
  svnAuthor: string | null
  svnRemoved: boolean
  /** 第一处冲突块，标签已换成「新基线（SVN）/ 你的提交」。二进制、太大、没有冲突标记时为 null。 */
  preview: string | null
}

/** 后端 `svn_follow::FollowOutcome`。 */
export type FollowOutcome =
  | { kind: 'done'; report: FollowReport }
  | {
      kind: 'needsManual'
      at: string
      files: string[]
      baselinePublished: boolean
      /** 分支没重放，但本地补丁可能已经上了服务器：用户要在动手之前知道。 */
      leaks: Leak[]
      /** 同上，但是在 TortoiseSVN 提交框里看过警告后提交的。旧后端没有。 */
      acknowledgedLeaks?: Leak[]
      /**
       * 不是重放冲突：分支落后、磁盘上还是这条分支的旧内容（`files`），只移分支会在「本地改动」里
       * 造出把 SVN 内容改回去的修改（Review 3 第 2 条）。`at` 是当前 HEAD。
       */
      staleDisk: boolean
      /** 手动跟进时会直接采用新基线版本的快照内容（6.10）：动手之前告诉用户。旧后端没有。 */
      snapshot?: [string, string[]][]
      /** 每个冲突文件的上下文（6.5），与 `files` 同序。旧后端没有。 */
      details?: ConflictDetail[]
    }
  /** 什么都没动：前置条件不满足，或者准备阶段失败。 */
  | { kind: 'refused'; code: string; message: string }

/**
 * 后端 `svn_follow::AutoFollowOutcome`（事故分析 6.12）。`skipped` 时一个 ref 都没动，`message` 直接
 * 给横幅：说清是哪一条条件没满足、点「跟进」会怎样。
 */
export type AutoFollowOutcome =
  | { kind: 'done'; report: FollowReport }
  | { kind: 'skipped'; code: string; message: string; conflict?: AutoFollowConflict }

/**
 * 后端 `svn_follow::AutoConflict`：被重放冲突拦下时的冲突文件。`message` 是给托盘的一整句，横幅按这里
 * 把冲突文件单独列成一组（2026-11-19 使用报告问题 1）。旧后端没有。
 */
export interface AutoFollowConflict {
  files: { path: string; rev: number | null; author: string | null; svnRemoved: boolean }[]
  /** 当前分支停在更早的基线上：冲突的文件可能不在这次的新内容里。 */
  lagging: boolean
}

/** 横幅上「为什么没自动跟进」的原因：一整句，被重放冲突拦下时另带冲突文件。 */
export interface AutoFollowSkip {
  message: string
  conflict?: AutoFollowConflict
  /**
   * 原因是部分跟进留下的文件（`heldBackScene`）。横幅不写这一行，留下的文件那一组（`describeHeldBack`）
   * 已经按原因列着；整句仍给任务栏与托盘，那里没有那一组（使用报告 2026-11-23）。
   */
  held?: boolean
}

/** 横幅上冲突文件那一组。 */
export interface AutoFollowConflictModel {
  title: string
  files: string[]
  note: string
}

/**
 * 冲突文件单独成组，排在新内容列表上面、用醒目的颜色。只是一行灰字的话，第一眼看到的是下面的新内容
 * 列表，像是列表里的文件冲突了（2026-11-19 使用报告问题 1）。文件行与新内容列表同一种写法。
 */
export function describeAutoFollowConflict(c: AutoFollowConflict): AutoFollowConflictModel {
  const files = c.files.map((f) => {
    if (f.svnRemoved) return `${f.path}  SVN 上已删除`
    if (f.rev == null) return f.path
    return `${f.path}  r${f.rev}${f.author ? `  ${f.author}` : ''}`
  })
  // 同一个意思后端也写了一份（`svn_follow::auto_blocker` 条件 7 的 `lag`），那句只给托盘；改一边记得改另一边
  const lag = c.lagging
    ? '这些文件不一定在下面的新内容里：当前分支还停在更早的基线上（切到了旧分支，或者上次跟进遇到了冲突），' +
      '重放要补上那之后 SVN 上的全部改动。'
    : ''
  return {
    // 重放的是提交，不是文件（独立 review）
    title: `没有自动跟进：当前分支的提交重放到新基线上，有 ${c.files.length} 个文件冲突`,
    files,
    note: `${lag}要你来判断，点「跟进」会弹出冲突的详情。`,
  }
}

/** 设置里关掉了自动跟进（后端 `commands::svn_sync::AUTO_FOLLOW_OFF`）。横幅照旧，不写原因。 */
export const AUTO_FOLLOW_OFF = 'svn_auto_follow_off'

/**
 * 这一次判定值不值得自动跟进一次，值得就返回它的「现场」键，否则 `null`。
 *
 * 只认 ExternallyAdvanced（日志 Task 1「偏离计划」：分支跟进入口不自动）。键由判定的全部内容与
 * HEAD 组成：同一个现场只试一次，不自动之后不会每次存盘、每次切回窗口都重跑一遍录制与服务器体检；
 * SVN 侧又有新内容、或者切了分支 / 提交了，键就变了，再试一次。
 */
export function autoFollowKey(
  sync: { verdict: { kind: string } } | null,
  head: string | null,
): string | null {
  if (!sync || sync.verdict.kind !== 'externallyAdvanced') return null
  return `${head ?? ''}\n${JSON.stringify(sync.verdict)}`
}

/** 横幅上「为什么没自动跟进」的那一行。 */
export function describeAutoFollowSkip(message: string): string {
  return `没有自动跟进：${message}`
}

/**
 * 被这些原因拦下时不记现场：原因在工作副本里，用户在 TortoiseSVN 里处理掉（Resolve）之后 HEAD 与判定
 * 都不变，现场键还是原来那个，记成「试过」就再也不自动了。不记的话，判定下一次写回时会再试一次；
 * 还拦着也只是多一遍 `svn status`（预检在录制与服务器体检之前就拒绝了）。
 */
export const RECHECK_SKIPS: ReadonlySet<string> = new Set(['svn_preflight_conflicted'])

/** 横幅原因里一组文件最多点名几个，同后端 `svn_follow::SUMMARY_NAMES`。 */
const HELD_NAMES = 5

/**
 * 部分自动跟进（计划 2026-11-13-partial-auto-follow）做完之后的现场是不是只剩它留下的那些文件。
 * 是就返回这个现场的键与横幅上的原因，调用方把它记成「试过」，不再自动发起一次。
 *
 * 不记的话：基线变了，现场键跟着变，自动跟进会立刻再跑一遍录制、服务器体检与重放，而结论一定
 * 还是被同样的文件拦下（留下它们的原因一条都没变），白等几秒，横幅上最后也只是多这一行原因。
 *
 * 判定里有任何别的文件（又来了新内容）、或者留下的文件已经不在判定里，现场就不是做完那一刻的了，
 * 返回 `null`，照常自动试。
 */
export function heldBackScene(
  held: HeldBack[] | undefined,
  sync: { verdict: { kind: string } } | null,
  head: string | null,
): { key: string; message: string; held: true } | null {
  const key = autoFollowKey(sync, head)
  if (!key || !sync || !held || held.length === 0) return null
  const v = sync.verdict as Extract<SyncVerdict, { kind: 'externallyAdvanced' }>
  if (v.added.length > 0 || v.removed.length > 0) return null
  const left = new Set(v.changed.map((c) => c.path))
  if (left.size !== held.length || !held.every((h) => left.has(h.path))) return null
  const byReason = new Map<string, string[]>()
  for (const h of held) byReason.set(h.reason, [...(byReason.get(h.reason) ?? []), h.path])
  const groups = [...byReason].map(([reason, paths]) => {
    const names = paths.slice(0, HELD_NAMES).join('、')
    const more = paths.length > HELD_NAMES ? ` 等 ${paths.length} 个文件` : ''
    // 原因是后端写的整句；万一哪条没带句号，也不和下一组粘在一起
    return `${names}${more}：${reason}${reason.endsWith('。') ? '' : '。'}`
  })
  return {
    key,
    // 横幅不写它（见 `AutoFollowSkip.held`），只给任务栏与托盘；前面会接上「没有自动跟进：」
    message:
      `还有 ${held.length} 个文件留在旧基线上（上一次自动跟进留下的），要你来处理。` +
      `${groups.join('')}点「跟进」来处理。`,
    held: true,
  }
}

/** 部分跟进留下的文件，钉在留下那一刻 SVN 侧的 revision 与 HEAD 上。 */
export interface HeldBackPin {
  /** 留下之后的 HEAD；`null`：没读出来，不按 HEAD 比。 */
  head: string | null
  files: HeldBackFile[]
}

export interface HeldBackFile {
  path: string
  reason: string
  /** 留下那一刻 SVN 侧的 revision；`null`：没读出来，只按路径认。 */
  rev: number | null
}

/**
 * 跟进完成后把留下的文件钉住，交给横幅一直列着（使用报告 2026-11-23）。
 *
 * 不能只靠 `heldBackScene` 的原因行：那一行只在现场只剩留下的文件时才有，又来了新内容、而这一次
 * 自动跟进没跑成（开关关了、merge driver 有风险、预检拦下）时，横幅上就没有任何地方说为什么留下。
 * 钉住 revision：SVN 侧那个文件又有了提交，原来的原因就不一定成立了，那时不再当成留下的。
 *
 * `scene` 为 `null`：读不出刷新之后的现场，照样记下，不按 revision 与 HEAD 比（少一层保护，
 * 但不至于把原因丢掉）。
 */
export function pinHeldBack(
  held: HeldBack[] | undefined,
  scene: { sync: { verdict: { kind: string } } | null; head: string | null } | null,
): HeldBackPin | null {
  if (!held || held.length === 0) return null
  if (!scene) return { head: null, files: held.map((h) => ({ ...h, rev: null })) }
  if (!scene.sync || scene.sync.verdict.kind !== 'externallyAdvanced') return null
  const v = scene.sync.verdict as Extract<SyncVerdict, { kind: 'externallyAdvanced' }>
  const revs = new Map(v.changed.map((c) => [c.path, c.rev]))
  const files = held.flatMap((h) => {
    const rev = revs.get(h.path)
    return rev === undefined ? [] : [{ path: h.path, reason: h.reason, rev }]
  })
  return files.length > 0 ? { head: scene.head, files } : null
}

/**
 * 当前 HEAD 下还作不作数。提交、切分支之后不作数：留下的原因（重放冲突、补丁泄漏）是那条分支、
 * 那个 HEAD 上的结论；宁可这一组暂时不显示，也不把别的分支的原因安到这里。
 */
export function heldBackFor(pin: HeldBackPin | null, head: string | null): HeldBackFile[] | null {
  if (!pin) return null
  return pin.head === null || pin.head === head ? pin.files : null
}

/** 横幅上留下的文件那一组。 */
export interface HeldBackModel {
  title: string
  /** 按原因分组，原因就是组的标题（后端 `svn_follow::HELD_*` 的整句）。 */
  groups: { reason: string; files: string[] }[]
  note: string
  /** 组里的文件行，横幅按整行把它们从新内容列表里摘掉，路径只出现一次。 */
  lines: Set<string>
}

/**
 * 部分跟进留下的文件单独成组，样式同整次冲突那一组（`describeAutoFollowConflict`），排在新内容
 * 列表上面。原来只是一行灰字，真实原因被上面那段通用说明盖住（使用报告 2026-11-23）。
 *
 * 只认还在判定里、revision 没变的文件；已经在整次冲突那一组里的不重复列。
 */
export function describeHeldBack(
  files: HeldBackFile[] | null | undefined,
  sync: { verdict: { kind: string } } | null,
  conflict?: AutoFollowConflict | null,
): HeldBackModel | null {
  if (!files || files.length === 0 || !sync || sync.verdict.kind !== 'externallyAdvanced') {
    return null
  }
  const v = sync.verdict as Extract<SyncVerdict, { kind: 'externallyAdvanced' }>
  const changed = new Map(v.changed.map((c) => [c.path, c]))
  const inConflict = new Set((conflict?.files ?? []).map((f) => f.path))
  const byReason = new Map<string, string[]>()
  const lines = new Set<string>()
  for (const f of files) {
    const c = changed.get(f.path)
    if (!c || inConflict.has(f.path) || (f.rev !== null && f.rev !== c.rev)) continue
    const line = externalChangeLine(c)
    byReason.set(f.reason, [...(byReason.get(f.reason) ?? []), line])
    lines.add(line)
  }
  if (lines.size === 0) return null
  return {
    title: `上一次自动跟进留下了 ${lines.size} 个文件，还停在旧基线上`,
    groups: [...byReason].map(([reason, list]) => ({ reason, files: list })),
    // 又有新内容会换一个现场键，`RepoTab` 的 effect 照常自动发起；设置里关掉了就不会（E2E 2026-11-24
    // 关着开关时看到原来那句「也会自动再试」是错的）
    note: '点「跟进」来处理；设置里开着自动的话，SVN 侧又有新内容时会再试一次。',
    lines,
  }
}

/** 自动跟进完成后那条不打断操作的提示（分析 §6.12「通知」）。详情是同一份跟进报告。 */
export interface AutoFollowNotice {
  /** `warn`：报告里有要你留意的内容（落后的分支、快照内容等），点开看。 */
  tone: 'quiet' | 'warn'
  title: string
  detail: string
}

export function describeAutoFollowDone(r: FollowReport): AutoFollowNotice {
  const m = describeFollowReport(r)
  // 放回的结果已经在 `safe` 里；没放回的会算进 todo。留在旧基线上的文件不算：下方横幅一直按原因
  // 列着它们，这里再算一项就是同一件事说两遍（使用报告 2026-11-23）。报告框里照旧算
  const todo = m.sections.filter((s) => s.level !== 'info' && s.id !== 'heldBack').length
  const look = todo > 0 ? `报告里有 ${todo} 项要你留意，请点「查看详情」。` : ''
  const below = (r.heldBack?.length ?? 0) > 0 ? '留下的文件和原因列在下面的横幅上。' : ''
  return {
    tone: todo > 0 ? 'warn' : 'quiet',
    title: '已自动跟进 SVN 侧的新内容',
    detail: `${m.summary}${m.safe}。${below}${look}`,
  }
}

/**
 * 新的一次跟进开始时，上一次「已自动跟进」的提示还留不留。
 *
 * 安静的提示只是告诉你「做完了」，留着会和新横幅打架（E2E 2026-11-13：旧提示写「放回了 1 个文件」，
 * 新横幅写「1 个文件缺 Local-Only 补丁」），收起。`warn` 的报告里有要你处理的内容（丢失的暂存、
 * 泄漏、落后的分支），「查看详情」是看它的唯一入口，用户没点「知道了」之前不替他扔掉（独立 review 中-1）。
 */
export function keepOnNewFollow(r: FollowReport | null): FollowReport | null {
  return r && describeAutoFollowDone(r).tone === 'warn' ? r : null
}

export type FollowActionId = 'followContinue'

/** 后端 `svn_follow_mark::SvnUserInfo`：「我的 SVN 用户名」的两个来源。 */
export interface SvnUserInfo {
  /** 配置里的 `svnUser`。 */
  configured: string | null
  /** `svn auth` 探测到的。 */
  detected: string | null
}

/**
 * 报告里一节的轻重，决定颜色与排序。
 *
 * - `danger`：不处理会出错，或者已经丢了东西（红）
 * - `attention`：要你确认一下（琥珀）
 * - `info`：只是告知（灰）
 */
export type ReportLevel = 'danger' | 'attention' | 'info'

const RANK: Record<ReportLevel, number> = { danger: 0, attention: 1, info: 2 }

/** 清单里的一行。路径与它的附注分开，才能让路径对齐、附注退到一边。 */
export interface ReportItem {
  /** 路径、分支名或 commit。 */
  text: string
  /** revision、所在分支与提交说明、落后的原因等。 */
  meta?: string
  /** 「…还有 N 个」那一行。 */
  more?: boolean
  /**
   * 能用外部工具对比时，要比的那个文件（仓库相对路径）。行数说不清差在哪，复杂的改动只能看 diff，
   * 所以「对不上」的每个文件旁边放一个「对比」（报告框决定有没有可用的工具）。
   */
  compare?: string
}

/** {@link describeFollowReport} 的一节。没发生的事不成节。 */
export interface FollowReportSection {
  id:
    | 'unverified'
    | 'leaks'
    | 'acknowledgedLeaks'
    | 'lost'
    | 'lostPartial'
    | 'superseded'
    | 'uncertain'
    | 'discarded'
    | 'merged'
    | 'changes'
    | 'dropped'
    | 'rewritten'
    | 'snapshot'
    | 'drift'
    | 'heldBack'
    | 'restorePending'
    | 'restored'
    | 'overlapDiffers'
    | 'overlapMatches'
    | 'moved'
    | 'behind'
  title: string
  /** 非 `info` 的节排在前面，见 {@link ReportLevel}。 */
  level: ReportLevel
  /** 截断之前的真实条数。 */
  count: number
  note?: string
  /** `label` 为空的组不画小标题。 */
  groups: { label: string; items: ReportItem[] }[]
}

export interface FollowReportModel {
  tone: NoticeTone
  /** 最重的一级，决定图标颜色。 */
  level: ReportLevel | 'ok'
  /** 要你留意的项数：非 `info` 的节数（没体检也是一节）。 */
  todo: number
  /** 结果写进标题：有几项要处理一眼就能看到。 */
  title: string
  summary: string
  /** 跟进不变的保证（工作区没被写），单独一行让人放心。 */
  safe: string
  /** 暂存与 commit 是按什么判定的（C1h）。 */
  basis: string
  /** 「跟进用时 …」，没量到时为 `null`。慢不慢要有个数：2026-11-11 那次的 70 秒只能靠估。 */
  elapsed: string | null
  sections: FollowReportSection[]
}

const MAX_LISTED = 20
/** 报告是可滚动的框，比结局提示放得下；根目录更新带进来的上万个文件仍然不能都进 DOM。 */
const MAX_REPORT_ITEMS = 200

function listed(items: string[]): string {
  const shown = items.slice(0, MAX_LISTED).map((f) => `· ${f}`)
  if (items.length > MAX_LISTED) shown.push(`…还有 ${items.length - MAX_LISTED} 个`)
  return shown.join('\n')
}

function capped(items: ReportItem[]): ReportItem[] {
  if (items.length <= MAX_REPORT_ITEMS) return items
  return [
    ...items.slice(0, MAX_REPORT_ITEMS),
    { text: `…还有 ${items.length - MAX_REPORT_ITEMS} 个`, more: true },
  ]
}

const plain = (items: string[]): ReportItem[] => items.map((text) => ({ text }))

/**
 * 泄漏提醒（事故分析 6.8）：原来一句「可能被提交到了 SVN，请检查」，用户分不清哪些已确认、哪些只是拿不准，
 * 也不知道它影不影响这次操作。按 hunk 判定分两档列，并写明影响。
 */
const LEAK_NOTE = '这些本地补丁（Local-Only）里有改动已经或可能出现在了 SVN 上'

/** 6.10 的快照内容：报告与冲突框共用的开头。 */
const SNAPSHOT_NOTE =
  '下面这些文件在 commit 里的内容与 SVN 上某个较新的版本逐字节相同（多半是 svn update 之后整包提交进 git 的）。' +
  '如果你是有意停在那个版本，请从原 commit 里取回。'

function leakLine(l: Leak): string {
  return `${l.path}（分支 ${l.branch}：${l.subject}）`
}

function leakItem(l: Leak): ReportItem {
  return { text: l.path, meta: `分支 ${l.branch}：${l.subject}` }
}

/** 重放后变空、已丢弃的提交上的泄漏：分支上已经没有它，不能再写成「分支 work：…」（计划 2026-11-16 的 A）。 */
function removedLeakItem(l: Leak): ReportItem {
  return { text: l.path, meta: `${l.subject}，原在分支 ${l.branch}` }
}

const REMOVED_LEAKS = '已从分支移除：内容都在 SVN 上了'

/**
 * 「要不要放回」一节的清单：原因只有一种时写进说明一次（返回 `reason`），每个文件后面只写补丁来自哪个
 * 提交与 SVN 最后提交者；原因各不相同时按原因分组，组名就是原因。
 */
function pendingRestoreGroups(
  items: { item: ReportItem; reason: string }[],
): { reason: string | null; groups: FollowReportSection['groups'] } {
  const reasons = [...new Set(items.map((i) => i.reason))]
  if (reasons.length <= 1) {
    const r = reasons[0] ?? ''
    return {
      // 原因是后端写的整句，接在说明中间；万一没带句号，也不和下一句粘在一起
      reason: r === '' ? null : r.endsWith('。') ? r : `${r}。`,
      groups: [{ label: '', items: items.map((i) => i.item) }],
    }
  }
  return {
    reason: null,
    groups: reasons.map((r) => ({
      label: r || '（没有原因）',
      items: items.filter((i) => i.reason === r).map((i) => i.item),
    })),
  }
}

/** SVN 上最后一次改它的提交：这次跟进到了才有（能放回的文件都在外部变更里，见 `drift_paths`）。 */
function svnLastCommit(r: FollowReport, path: string): string | null {
  const c = r.changes.find((x) => x.path === path)
  if (!c) return null
  return `SVN 最后提交：${c.author ?? '没有作者'}（r${c.rev}）`
}

/** 所有 hunk 都原样出现在新基线里的算「已确认」；有任何一处对不上的算「拿不准」。 */
function leakTiers(leaks: Leak[]): { sure: Leak[]; unsure: Leak[] } {
  const isSure = (l: Leak) => l.hunks.length > 0 && l.hunks.every((h) => h.verdict === 'present')
  return { sure: leaks.filter(isSure), unsure: leaks.filter((l) => !isSure(l)) }
}

/**
 * 提交时看过警告的泄漏：用户在 TortoiseSVN 提交框里已经确认过一次，这里只留档（设计
 * 2026-11-16-tsvn-acknowledged-leaks）。
 */
const ACKED_TITLE = '提交时已确认的 Local-Only 改动'
const ACKED_NOTE =
  '这些 Local-Only 改动是你在 TortoiseSVN 提交框里看过 jkk 的警告、点了 Proceed anyway 提交的，' +
  '所以这次没有拦下跟进。只是留个记录，不用处理。'

function ackedLeakSection(leaks: Leak[]): string {
  return `${ACKED_TITLE}：${ACKED_NOTE}\n${listed(leaks.map(leakLine))}`
}

function leakSection(leaks: Leak[]): string {
  const { sure, unsure } = leakTiers(leaks)
  const parts = [`${LEAK_NOTE}。只是提醒，不拦手动跟进，也不改任何文件。`]
  if (sure.length > 0) parts.push(`这些改动已确认在 SVN 上：\n${listed(sure.map(leakLine))}`)
  if (unsure.length > 0) {
    parts.push(`拿不准（SVN 动过同一处，但对不上）：\n${listed(unsure.map(leakLine))}`)
  }
  return parts.join('\n')
}

/** 怎么找回被换掉的暂存。索引没有 reflog，这个 ref 是唯一的出路（分析 §6.11.1）。 */
function pinnedNote(ref: string | null): string {
  const r = ref ?? 'refs/mygitclient/follow/<opid>'
  return (
    `原内容钉在 ${r} 下，只保留最近几次跟进的。` +
    `需要时用 git ls-tree -r ${r} 列出来，再用 git show ${r}:blobs/<oid> 取回。`
  )
}

/** 按作者分组：看一眼就知道哪些是别人的改动（分析 §6.10，U7）。作者按名字排，匿名排在后面。 */
function changeGroups(r: FollowReport): FollowReportSection['groups'] {
  const byAuthor = new Map<string, ReportItem[]>()
  const anonymous: ReportItem[] = []
  for (const c of r.changes) {
    const line = { text: c.path, meta: `r${c.rev}` }
    if (!c.author) anonymous.push(line)
    else byAuthor.set(c.author, [...(byAuthor.get(c.author) ?? []), line])
  }
  const named = [...byAuthor.keys()].sort().map((a) => ({ label: a, items: byAuthor.get(a)! }))
  return [
    ...named,
    { label: '（没有作者）', items: anonymous },
    { label: '已在 SVN 上删除', items: plain(r.removed) },
  ]
}

const commitLabel = (c: string) => `commit ${c.slice(0, 10)}`

/**
 * 跟进报告的展示模型（分析 §6.10、§6.12）。
 *
 * 要用户留意的节排在最前：根目录更新一次能带进来几千个文件，排在文件清单后面就等于看不见，
 * 而泄漏与丢失的暂存只在这份报告里出现一次。
 */
export function describeFollowReport(r: FollowReport): FollowReportModel {
  const merged = Object.entries(r.stage.decisions)
    .filter(([, d]) => typeof d === 'object')
    .map(([path]) => path)
  const pinned = pinnedNote(r.pinnedRef)
  // 旧后端没有这两个字段
  const snapshot = r.snapshot ?? []
  // 能放回的单列成 Local-Only 那两节：留在「对不上」里，文案会让用户把磁盘版本提交进分支，
  // 照做补丁就丢了（使用报告 2026-11-12 缺陷 5）
  const restorable = r.restorable ?? []
  const drift = (r.drift ?? []).filter((p) => !restorable.includes(p))
  const restored = r.restore?.restored ?? []
  const commitsOf = (p: string) => (r.restore?.commits?.[p] ?? []).join('、')
  const reasonOf = new Map((r.restore?.skipped ?? []).map((k) => [k.path, k.reason]))
  const notRestored = restorable
    .filter((p) => !restored.includes(p))
    .map((p) => {
      const c = commitsOf(p)
      const parts = [c ? `补丁来自 ${c}` : '', svnLastCommit(r, p) ?? ''].filter((t) => t !== '')
      const item: ReportItem = parts.length > 0 ? { text: p, meta: parts.join(' · ') } : { text: p }
      return { item, reason: reasonOf.get(p) ?? '' }
    })
  const pending = pendingRestoreGroups(notRestored)
  const overlap = r.overlap ?? []
  const commitsByPath = new Map(overlap.map((o) => [o.path, o.commits.join('、')]))
  const svnRemoved = new Set(r.removed)
  const driftItem = (p: string): ReportItem => {
    const c = commitsByPath.get(p)
    const parts = [
      driftLinesText(r.driftLines?.[p], svnRemoved.has(p)),
      c ? `改过它的提交：${c}` : null,
    ]
    const meta = parts.filter((t): t is string => t !== null).join(' · ')
    const item: ReportItem = meta ? { text: p, meta } : { text: p }
    return r.driftRange ? { ...item, compare: p } : item
  }
  // 重放后变空、已丢弃的提交上的泄漏单列一组：它已经不在分支上了（计划 2026-11-16 的 A）。
  // 只认移动了的分支：别的分支带着同一个提交时，`plan_other` 见泄漏就停，那条分支没重放、提交还在，
  // 而且一定进了 behindBranches。报告里没有当前分支名，只能这样排除
  const gone = new Set(r.dropped)
  const stayed = new Set(r.behindBranches.map(([b]) => b))
  const isRemoved = (l: Leak) => gone.has(l.commit) && !stayed.has(l.branch)
  const removedLeaks = r.leaks.filter(isRemoved)
  const acked = r.acknowledgedLeaks ?? []
  const overlapItem = (o: OverlapFile): ReportItem => ({ text: o.path, meta: o.commits.join('、') })
  // drift 里的已经在「分支与磁盘对不上」那一节，不重复列
  // 后端把整个 drift（含能放回的）并进了 overlap。能放回的已经在 Local-Only 那两节，按原始 drift
  // 排除；只排除剩下的 drift 会把它们漏进这一节，那里的文案同样建议以磁盘为准提交（review H1）
  const differs = overlap.filter((o) => !o.matchesDisk && !(r.drift ?? []).includes(o.path))
  const matches = overlap.filter((o) => o.matchesDisk)
  const partial = r.stage.partial ?? {}
  // 不用 `in`：根目录下叫 `constructor` 的文件会命中原型链，真丢的暂存被降一级（独立 review L1）
  const hasPartial = (p: string) => Object.prototype.hasOwnProperty.call(partial, p)
  const lostAll = r.stage.lost.filter((p) => !hasPartial(p))
  const lostPart = r.stage.lost
    .filter(hasPartial)
    .map((p) => ({
      text: p,
      meta: `${partial[p].total} 处改动里 ${partial[p].present} 处已在 SVN 上`,
    }))
  const { sure, unsure } = leakTiers(r.leaks.filter((l) => !isRemoved(l)))
  // 顺序就是展示顺序：先 danger，再 attention，最后 info；info 里与这次跟进最相关的在前
  const candidates: Omit<FollowReportSection, 'count'>[] = [
    {
      id: 'drift',
      title: '分支与磁盘对不上',
      level: 'danger',
      // 三句：是什么、怎么办、例外。能放回的已经单列，例外只在后端没挑出它们时（出错）才用得上
      note:
        '这些文件分支里的内容和磁盘上的 SVN 版本不同（或者分支把 SVN 已删的文件带了回来），' +
        '「本地改动」里会出现它们。磁盘上的版本是对的，就把它提交进分支。' +
        '例外：差的是 Local-Only 补丁时别提交，那会把补丁删掉。',
      groups: [{ label: '', items: drift.map(driftItem) }],
    },
    {
      id: 'heldBack',
      title: '留在旧基线上的文件',
      level: 'attention',
      note:
        '这些文件 SVN 侧的新内容这次没有跟进，原因写在每个文件后面；其余文件照常跟进了。' +
        '顶部横幅会继续列出它们：处理好之后点「跟进」，或者等下一次自动跟进。',
      groups: [
        {
          label: '',
          items: (r.heldBack ?? []).map((h) => ({ text: h.path, meta: h.reason })),
        },
      ],
    },
    {
      id: 'restorePending',
      // 标题就是要用户做的决定（计划 2026-11-16 的 A）：原来写「没放回」，下面却给放回按钮
      title: `要不要放回这 ${notRestored.length} 个文件的 Local-Only 补丁？`,
      level: 'attention',
      // 「磁盘上是 SVN 的版本」只在跟进那一刻成立：没放回的原因可能正是磁盘或暂存区里另有内容，所以
      // 不叫人提交，「不要补丁」这条路与顶部横幅一致，点「不放回」（独立 review 第 2、3 条）
      note:
        '跟进时，这些文件分支比磁盘只多 Local-Only 补丁，磁盘上是 SVN 的版本。' +
        (pending.reason ?? '') +
        '要补丁就放回：SVN 里它们会显示为已修改，git 里是干净的；关掉报告之后，顶部横幅上也能放回。' +
        '是你有意去掉的，就在顶部横幅上点「不放回」。',
      groups: pending.groups,
    },
    {
      id: 'leaks',
      title: '泄漏告警',
      level: 'danger',
      note:
        `${LEAK_NOTE}。只是提醒，不影响这次跟进；` +
        '带着这些补丁的其他分支不会自动移动，切过去时会提示手动跟进。确认是你有意提交的，就可以不管它。' +
        (removedLeaks.length > 0
          ? '「已从分支移除」的提交重放后变空，已经丢弃，分支上不会再有它们。'
          : ''),
      groups: [
        { label: '这些改动已确认在 SVN 上', items: sure.map(leakItem) },
        { label: '拿不准', items: unsure.map(leakItem) },
        { label: REMOVED_LEAKS, items: removedLeaks.map(removedLeakItem) },
      ],
    },
    {
      id: 'lost',
      title: '丢失的暂存',
      level: 'danger',
      note: `这些暂存内容没能保留，已取 SVN 的版本。${pinned}`,
      groups: [{ label: '', items: plain(lostAll) }],
    },
    {
      id: 'lostPartial',
      title: '部分已在 SVN 上的暂存',
      level: 'attention',
      note:
        '这些暂存和 SVN 的新内容合不到一起，已取 SVN 的版本；暂存里有一部分改动 SVN 上已经有了，' +
        `其余的请对照原内容确认还要不要。${pinned}`,
      groups: [{ label: '', items: lostPart }],
    },
    {
      id: 'uncertain',
      title: '拿不准的暂存',
      level: 'attention',
      note: '这些暂存原样保留：分不清它和你的提交谁先谁后，请确认。不需要的话取消暂存即可。',
      groups: [{ label: '', items: plain(r.stage.uncertain) }],
    },
    {
      id: 'snapshot',
      title: '按基线对齐的快照内容',
      level: 'attention',
      note: `${SNAPSHOT_NOTE}重放时直接采用了新基线的版本。`,
      groups: snapshot.map(([c, paths]) => ({ label: commitLabel(c), items: plain(paths) })),
    },
    {
      id: 'overlapDiffers',
      title: '提交与 SVN 都改过、和磁盘不一致的文件',
      level: 'attention',
      note:
        '分支上的提交改过这些文件，SVN 侧也有新内容，重放出来的分支版本和磁盘不一致，' +
        '「本地改动」里显示的就是两者的差别。那是你还没提交的改动就不用管；' +
        '如果出现了不是你做的改动（比如把同事的改动改回去），说明重放合错了，请以磁盘为准提交进分支。',
      groups: [{ label: '', items: differs.map(overlapItem) }],
    },
    {
      id: 'behind',
      title: '落后的分支',
      level: 'attention',
      note: '这些分支没有移动，切过去时会提示手动跟进。',
      groups: [
        { label: '', items: r.behindBranches.map(([b, why]) => ({ text: b, meta: why })) },
      ],
    },
    // 排在文件清单前面：清单可能上千行，留档也要看得到
    {
      id: 'acknowledgedLeaks',
      title: ACKED_TITLE,
      level: 'info',
      note: ACKED_NOTE,
      groups: [
        { label: '', items: acked.filter((l) => !isRemoved(l)).map(leakItem) },
        { label: REMOVED_LEAKS, items: acked.filter(isRemoved).map(removedLeakItem) },
      ],
    },
    { id: 'changes', title: '跟进的文件', level: 'info', groups: changeGroups(r) },
    {
      id: 'moved',
      title: '一起移动的分支',
      level: 'info',
      note: '这些分支一起重放到了新基线上，没有动工作区。',
      groups: [{ label: '', items: plain(r.movedBranches) }],
    },
    {
      id: 'restored',
      title: '已放回的 Local-Only 补丁',
      level: 'info',
      note: '跟进之后把这些补丁写回了磁盘：git 里是干净的，SVN 里它们会显示为已修改。',
      groups: [
        {
          label: '',
          items: restored.map((p) => {
            const c = commitsOf(p)
            return c ? { text: p, meta: c } : { text: p }
          }),
        },
      ],
    },
    {
      id: 'overlapMatches',
      title: '提交与 SVN 都改过、和磁盘一致的文件',
      level: 'info',
      note: '分支上的提交改过这些文件，SVN 侧也有新内容；重放出来的分支版本与磁盘逐字节相同，不用处理。',
      groups: [{ label: '', items: matches.map(overlapItem) }],
    },
    {
      id: 'merged',
      title: '合并的暂存',
      level: 'info',
      note: '这些暂存和 SVN 的新内容做了三方合并，结果留在暂存区。',
      groups: [{ label: '', items: plain(merged) }],
    },
    {
      id: 'superseded',
      title: '已被 SVN 取代的暂存',
      level: 'info',
      note: `这些暂存的改动 SVN 的新版本里都已经有了，暂存区取了 SVN 的版本，没有丢内容。${pinned}`,
      groups: [{ label: '', items: plain(r.stage.superseded ?? []) }],
    },
    {
      id: 'discarded',
      title: '丢弃的暂存',
      level: 'info',
      note: `这些暂存是已提交版本之前的中间版本，已丢弃。${pinned}`,
      groups: [{ label: '', items: plain(r.stage.discarded) }],
    },
    {
      id: 'rewritten',
      title: '换成 SVN 版本的路径',
      level: 'info',
      note: '这些路径你已经提交到了 SVN，commit 里的版本让位给 SVN 的版本。',
      groups: r.rewritten.map(([c, paths]) => ({ label: commitLabel(c), items: plain(paths) })),
    },
    {
      id: 'dropped',
      title: '丢弃的 commit',
      level: 'info',
      note: '这些 commit 的内容都已经在 SVN 上，重放之后变空，不再保留。',
      groups: [{ label: '', items: plain(r.dropped.map((c) => c.slice(0, 10))) }],
    },
  ]
  const listed = candidates
    .map((s) => {
      const groups = s.groups.filter((g) => g.items.length > 0)
      return {
        ...s,
        count: groups.reduce((n, g) => n + g.items.length, 0),
        groups: groups.map((g) => ({ ...g, items: capped(g.items) })),
      }
    })
    .filter((s) => s.groups.length > 0)
  // 没有清单，只有一句话；排在最前：整份报告的前提有问题
  const unverified: FollowReportSection[] = r.verified
    ? []
    : [
        {
          id: 'unverified',
          title: '新基线没向服务器体检',
          level: 'danger',
          count: 0,
          note: '这次没能向服务器体检新基线（可能是断网），基线照样发布了。连上之后建议手动体检一次。',
          groups: [],
        },
      ]
  // 按轻重排，同级保持书写顺序（sort 是稳定的）：以后加候选时写错位置，也不会排到文件清单后面去
  const sections = [...unverified, ...listed].sort((a, b) => RANK[a.level] - RANK[b.level])

  const todo = sections.filter((s) => s.level !== 'info').length
  const danger = sections.some((s) => s.level === 'danger')
  const level = danger ? 'danger' : todo > 0 ? 'attention' : 'ok'

  const count = r.changes.length + r.removed.length
  const facts = [count > 0 ? `跟进了 ${count} 个文件` : '分支已跟上基线']
  if (r.movedBranches.length > 0) facts.push(`${r.movedBranches.length} 条分支一起移动`)
  const held = r.heldBack?.length ?? 0
  if (held > 0) facts.push(`${held} 个文件留在旧基线上`)
  facts.push(`基线最低在 r${r.newFloor}`)

  return {
    tone: level === 'ok' ? 'ok' : 'attention',
    level,
    todo,
    title:
      todo === 0
        ? '跟进完成'
        : `跟进完成，有 ${todo} 项${danger ? '需要处理' : '请确认'}`,
    summary: `${facts.join('，')}。`,
    safe:
      restored.length > 0
        ? `工作区里只放回了 ${restored.length} 个文件的 Local-Only 补丁，其余文件没有改动`
        : '工作区里的文件没有改动',
    basis:
      r.evidence === 'hooks' ? '判定依据：TortoiseSVN 钩子事件' : '判定依据：作者 + 失焦快照',
    elapsed: r.elapsedMs === undefined ? null : `跟进用时 ${formatDuration(r.elapsedMs)}`,
    sections,
  }
}

/**
 * 跟进报告里要不要问「我的 SVN 用户名」（日志 D4）。
 *
 * 有拿不准的暂存时总要问：探测到的名字可能不是提交时用的那个，这正是判断落空的地方之一。
 */
export function needsSvnUserPrompt(r: FollowReport, info: SvnUserInfo): boolean {
  return r.stage.uncertain.length > 0 || (!info.configured && !info.detected)
}

export function describeFollowContinue(): SyncConfirmModel {
  return {
    title: '继续跟进',
    message:
      '把上次中断的跟进走完。走不下去时会先把分支移回原处，再重新跟进一次。不会写工作区里的文件。',
    confirmLabel: '继续',
    busyLabel: '正在继续…',
  }
}

/**
 * 冲突文件一行一个，带上 SVN 侧是谁、哪个 revision 改的（事故分析 6.5：11-05 要手工跑 svn log 才看清）。
 * 旧后端没有 `details` 时照原样列文件名。
 */
function conflictLines(o: Extract<FollowOutcome, { kind: 'needsManual' }>): string[] {
  const byPath = new Map((o.details ?? []).map((d) => [d.path, d]))
  return o.files.map((f) => {
    const d = byPath.get(f)
    if (!d) return f
    if (d.svnRemoved) return `${f}（SVN 上已删除）`
    if (d.svnRev === null) return f
    return `${f}（SVN 上 r${d.svnRev}${d.svnAuthor ? ` ${d.svnAuthor}` : ''} 改过）`
  })
}

/** 冲突那几行的预览：后端只给前几个文件，每个一段。 */
function previewSection(details: ConflictDetail[]): string[] {
  const shown = details.filter((d) => d.preview)
  if (shown.length === 0) return []
  return [
    '冲突的地方（只列前几个文件的第一处）：\n\n' +
      shown.map((d) => `${d.path}\n${d.preview}`).join('\n\n'),
  ]
}

/**
 * 事故分析 6.7：手动跟进没有单独的按钮，取消之后用户找不到回来的路。按钮上写的是 Cancel，文案照着写。
 */
const COME_BACK_NOTE =
  '现在不想处理也可以点「Cancel」，什么都不会动。之后随时点顶部横幅上的「跟进」，会回到这一步。'

/** 跟进没走完时说什么。走完了说什么在 {@link describeFollowReport}。 */
export function describeFollowOutcome(o: Exclude<FollowOutcome, { kind: 'done' }>): OutcomeNotice {
  switch (o.kind) {
    case 'refused':
      return {
        tone: 'attention',
        title: '这次没有跟进',
        detail: `${o.message}\n\n分支、基线和暂存区都没有改动。`,
      }
    case 'needsManual': {
      const sections = o.staleDisk
        ? [
            '基线已经发布，但当前分支落后基线，而下面这些文件磁盘上还是这条分支的旧内容' +
              `（git 看着没改，TortoiseSVN 看着已修改）：\n${listed(o.files)}`,
            '只移动分支的话，这些文件会在「本地改动」里显示成把 SVN 上的内容改回去，提交就等于回退。' +
              '分支和暂存区都没动。点「手动跟进」：未提交的改动先收起来，分支重放到新基线上时会把新内容' +
              '写回磁盘，最后再把改动放回来。这一步会写工作区里的文件。',
            COME_BACK_NOTE,
          ]
        : [
            `基线已经发布，但把提交 ${o.at.slice(0, 10)} 重放上去时有冲突：\n${listed(conflictLines(o))}`,
            ...previewSection(o.details ?? []),
            '分支和暂存区都没动。这需要人来判断：点「手动跟进」，未提交的改动会先收起来，分支重放到' +
              '新基线上，冲突由你解决，最后再把改动放回来。这一步会写工作区里的文件。',
            COME_BACK_NOTE,
          ]
      const snapshot = (o.snapshot ?? []).flatMap(([c, paths]) =>
        paths.map((p) => `${p}（${c.slice(0, 10)}）`),
      )
      if (snapshot.length > 0) {
        sections.push(
          `${SNAPSHOT_NOTE}手动跟进时这些文件会直接采用新基线的版本，不会报冲突：\n${listed(snapshot)}`,
        )
      }
      if (o.leaks.length > 0) sections.push(leakSection(o.leaks))
      const acked = o.acknowledgedLeaks ?? []
      if (acked.length > 0) sections.push(ackedLeakSection(acked))
      return {
        tone: 'attention',
        title: o.staleDisk ? '这次要手动跟进' : '跟进遇到冲突',
        detail: sections.join('\n\n'),
      }
    }
  }
}

/**
 * 手动跟进的确认框。**它是唯一会写磁盘的跟进**，所以永远由用户点了这个框才走。
 */
export function describeManualFollow(): SyncConfirmModel {
  return {
    title: '手动跟进',
    message:
      '把分支重放到已经发布的基线上：未提交的改动先收起来，冲突由你解决，最后把改动放回来。' +
      '不切到基线，也不跑 svn update，基线保持原样。',
    warning:
      '这一步会写工作区里的文件，Unity 之类的工具可能会重新导入。' +
      '暂存状态会尽量保住；保不住时改动内容都在，暂存的那一份留在 stash 里，结束时会告诉你。',
    confirmLabel: '手动跟进',
    busyLabel: '正在手动跟进…',
  }
}

/**
 * 跟进冲突时的确认框：冲突的事实（含泄漏告警）放在手动跟进的确认框上，按钮就是出路，
 * 不必先看一个「知道了」再去找入口。
 */
export function describeFollowConflict(
  o: Extract<FollowOutcome, { kind: 'needsManual' }>,
): SyncConfirmModel {
  const n = describeFollowOutcome(o)
  return { ...describeManualFollow(), title: n.title, message: n.detail }
}

/**
 * 切到落后基线的分支（分析 §6.11.3）。默认「切换并手动跟进」；「仅切换」的风险写在警告条里，
 * 与 §6.5 F5 ④ 是同一件事。
 *
 * 没有 `busyLabel`：切分支不走这个框的 `action`（切完还要接着弹手动跟进的框，见 `svnSyncFlow`），
 * 框点完就关，没有忙碌态（Review 3 第 4 条）。
 */
export function describeCheckoutBehind(
  branch: string,
): Omit<SyncConfirmModel, 'busyLabel'> & { alternativeLabel: string } {
  return {
    title: '切换到落后基线的分支',
    message:
      `分支 ${branch} 还停在更早的基线上。「切换并手动跟进」切过去之后接着把它重放到新基线上，` +
      '未提交的改动会先收起来，冲突由你解决。',
    warning:
      '「仅切换」的风险：这条分支里还是旧内容，切过去时 git 会把旧内容写回磁盘，' +
      'TortoiseSVN 随即把它们显示成「已修改」。勾上提交的话，等于回退了已经在 SVN 上的内容。',
    confirmLabel: '切换并手动跟进',
    alternativeLabel: '仅切换',
  }
}

/** 横幅上「继续 / 放弃」的确认框。按钮沿用更新，话不能照搬：这里没有「回到更新前」这回事。 */
export function describeManualFollowAction(id: SyncActionId, phase: string): SyncConfirmModel {
  if (id === 'syncContinue') {
    const message =
      phase === 'rebase'
        ? '确认冲突文件都已改好并暂存。会接着把剩下的提交重放完，再放回未提交的改动。'
        : phase === 'applying-stash'
          ? '确认冲突已解决（或同名文件已挪走）。会结束这次手动跟进；如果刚才放回时有冲突，原始备份会留在 stash 列表里，由你确认后自己删。'
          : '从中断的地方接着走完这次手动跟进。'
    return { title: '继续手动跟进', message, confirmLabel: '继续', busyLabel: '正在继续…' }
  }
  const message =
    phase === 'start'
      ? '上次手动跟进在动手之前就中断了，清除这条记录即可，不会改动任何文件。'
      : phase === 'rebase'
        ? '中止重放，分支回到手动跟进之前的位置，放回未提交的改动。基线保持在新的位置，你的分支仍然落后于它。'
        : phase === 'applying-stash'
          ? '结束这次手动跟进，不再自动放回改动，分支留在新基线上。stash 里的备份（mygitclient-sync:…）会留在 stash 列表里，你可以之后自己取回。'
          : '回到手动跟进之前的分支，放回未提交的改动。基线保持在新的位置。'
  return {
    title: '放弃这次手动跟进',
    message,
    confirmLabel: '放弃',
    busyLabel: '正在放弃…',
    danger: phase !== 'start',
  }
}

/** 手动跟进的结局。与更新同一组结局，话要按它自己的事实说：基线没有在这次里前进过。 */
export function describeManualFollowOutcome(o: SyncInOutcome): OutcomeNotice | null {
  const short = 'stashOid' in o && o.stashOid ? o.stashOid.slice(0, 10) : ''
  switch (o.kind) {
    case 'done':
      return null // 成功不打扰：界面刷新本身就是结果
    case 'aborted':
      return {
        tone: 'ok',
        title: '已放弃这次手动跟进',
        detail: '分支回到了手动跟进之前的位置，未提交的改动和暂存都已放回。基线保持在新的位置。',
      }
    case 'stageLost':
      return {
        tone: 'attention',
        title:
          o.newRev === null ? '已放弃这次手动跟进，暂存状态没保住' : '手动跟进完成，暂存状态没保住',
        detail: (o.newRev === null ? '分支回到了手动跟进之前的位置。' : '') + stageLostDetail(o),
      }
    case 'rebaseConflict':
      return {
        tone: 'attention',
        title: '手动跟进遇到冲突',
        detail:
          `把你的提交重放到新基线上时有冲突：\n${listed(o.files)}\n\n` +
          '在「本地改动」里改好这些文件并暂存，然后点顶部横幅上的「继续」。想回到手动跟进之前，点「放弃」。',
      }
    case 'stashConflict':
      return {
        tone: 'attention',
        title: '放回未提交的改动时有冲突',
        detail:
          `分支已经重放到新基线上，但你未提交的改动与它冲突了，「已暂存」的状态也没保住：\n${listed(o.files)}\n\n` +
          `改好并暂存之后点顶部横幅上的「继续」。改动的原始备份一直在 stash 里（${short}）。`,
      }
    case 'stashBlocked':
      return {
        tone: 'attention',
        title: '未提交的改动还没放回来',
        detail:
          `磁盘上已经有与你的未跟踪文件同名的文件，直接放回会覆盖其中一边：\n${listed(o.collisions)}\n\n` +
          `把这些文件挪到别处（或改名）之后点顶部横幅上的「继续」。一个字节都还没放回，备份在 stash ${short}。`,
      }
    case 'released':
      return {
        tone: 'attention',
        title: o.rolledBack ? '已放弃这次手动跟进，改动没有自动放回' : '手动跟进已结束，改动没有自动放回',
        detail:
          (o.rolledBack ? '分支回到了手动跟进之前的位置。' : '分支已经在新基线上。') +
          '未提交的改动没有（完整）放回来，如果刚才有冲突，文件里可能还留着冲突标记，请自己处理。' +
          `原始备份一直在 stash 列表里（mygitclient-sync:…，${short}），需要时自己取回，不要当成垃圾清掉。`,
      }
    case 'settled':
      return {
        tone: 'ok',
        title: o.rolledBack ? '已放弃这次手动跟进' : '手动跟进完成',
        detail:
          (o.rolledBack ? '分支回到了手动跟进之前的位置，' : '') +
          '未提交的改动按你处理后的样子留在工作区里。' +
          `为防万一，改动的原始备份没有删，还在 stash 列表里（mygitclient-sync:…，${short}）。` +
          '确认工作区里的内容没问题之后，可以自己把它删掉。',
      }
    case 'followDrift':
      return describeFollowDrift(o)
  }
}

/** 后端 `svn_restore_local_only::RestoreSkip`。 */
export interface RestoreSkip {
  path: string
  /** 给人看的原因。 */
  reason: string
}

/** 后端 `svn_restore_local_only::RestoreOutcome`：「放回 Local-Only 补丁」的结果。 */
export interface RestoreOutcome {
  restored: string[]
  skipped: RestoreSkip[]
  /** 路径 → 改过它的 Local-Only 提交（`短 hash 说明`）。旧后端没有。 */
  commits?: Record<string, string[]>
}

/** 后端 `svn_restore_local_only::PendingRestore`：横幅上一个缺 Local-Only 补丁的文件。 */
export interface PendingRestore {
  path: string
  /** 改过它的 Local-Only 提交（`短 hash 说明`）。 */
  commits: string[]
}

/**
 * 报告里还能点按钮放回的文件：能放回、跟进时又没自动放回的。旧后端没有 `restore`，全部算没放回。
 */
export function restorablePaths(r: FollowReport): string[] {
  const done = r.restore?.restored ?? []
  return (r.restorable ?? []).filter((p) => !done.includes(p))
}

/**
 * 横幅上点「放回」后没写成的文件，给错误条的一句话；全部写成时 `null`。不挂在横幅上：再等也不会
 * 满足的文件会被移出清单，横幅随之消失，原因就看不到了（review L2）。
 */
export function describeRestoreSkips(o: RestoreOutcome): string | null {
  if (o.skipped.length === 0) return null
  const each = o.skipped.map((k) => `${k.path}（${k.reason}）`).join('；')
  return `有 ${o.skipped.length} 个文件没放回 Local-Only 补丁：${each}`
}

/** 横幅「N 个文件缺 Local-Only 补丁」（计划 2026-11-12-auto-restore-local-only 的 C）。空清单不挂。 */
export function describePendingRestores(
  list: PendingRestore[],
): { title: string; detail: string; files: string[] } | null {
  if (list.length === 0) return null
  return {
    title: `${list.length} 个文件缺 Local-Only 补丁`,
    detail:
      '跟进之后这些文件的磁盘上是 SVN 的版本，分支里还多着 Local-Only 补丁，「本地改动」里显示成删掉了补丁。' +
      '不要把它提交进分支，那会把补丁删掉。点「放回」把补丁写回磁盘；如果是你有意去掉的，点「不放回」。',
    files: list.map((p) => (p.commits.length > 0 ? `${p.path} ← ${p.commits.join('、')}` : p.path)),
  }
}

/** 「放回 Local-Only 补丁」之后在报告里说什么。`ok` 为 false 时要用户留意跳过的文件。 */
export function describeRestoreOutcome(o: RestoreOutcome): { ok: boolean; text: string } {
  const done = o.restored.length
  const skipped = o.skipped.length
  if (skipped === 0) {
    return {
      ok: true,
      text: `已放回 ${done} 个文件的 Local-Only 补丁。SVN 里它们会显示为已修改，git 里是干净的。`,
    }
  }
  const head = done > 0 ? `已放回 ${done} 个文件，` : '一个文件都没放回，'
  return { ok: false, text: `${head}${skipped} 个跳过了：` }
}
