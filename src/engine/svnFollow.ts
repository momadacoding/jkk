/**
 * 跟进外部提交（分析 §6.5）与手动跟进（计划 Task 13）的类型与文案。
 *
 * 纯函数，无 React / Tauri 依赖（`AGENTS.md` §7.2）。跟进完成时说什么在 {@link describeFollowReport}，
 * 没完成时说什么在 {@link describeFollowOutcome}。
 *
 * ⚠️ 横幅与对话框都按**纯文本**渲染：不要写 `**`。
 */
import type { ExternalChange } from './svnSync'
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
   * `drift` 里能用「放回 Local-Only 补丁」写回磁盘的（使用报告 9.1）：分支比磁盘多出来的只有
   * Local-Only 改动，磁盘就是 SVN 的版本。旧后端没有。
   */
  restorable?: string[]
  /** 条件 11 的重叠文件，逐个标出是否与磁盘一致。旧后端没有。 */
  overlap?: OverlapFile[]
  stage: ComposeReport
  movedBranches: string[]
  behindBranches: [string, string][]
  leaks: Leak[]
  /** 钉着换入之前的原始索引与暂存 blob 的 ref。 */
  pinnedRef: string | null
  /** 判定依据（C1h）：钩子健康时按本机事件，否则按作者 + 失焦快照。 */
  evidence: 'hooks' | 'heuristic'
  /** 读收件箱的时刻。前端不用，随 journal 走。 */
  evidenceCutoffMs: number | null
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
  | { kind: 'skipped'; code: string; message: string }

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

/** 自动跟进完成后那条不打断操作的提示（分析 §6.12「通知」）。详情是同一份跟进报告。 */
export interface AutoFollowNotice {
  /** `warn`：报告里有要你留意的内容（落后的分支、快照内容等），点开看。 */
  tone: 'quiet' | 'warn'
  title: string
  detail: string
}

export function describeAutoFollowDone(r: FollowReport): AutoFollowNotice {
  const m = describeFollowReport(r)
  const look = m.tone === 'ok' ? '' : `报告里有 ${m.todo} 项要你留意，请点「查看详情」。`
  return {
    tone: m.tone === 'ok' ? 'quiet' : 'warn',
    title: '已自动跟进 SVN 侧的新内容',
    detail: `${m.summary}${m.safe}。${look}`,
  }
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
}

/** {@link describeFollowReport} 的一节。没发生的事不成节。 */
export interface FollowReportSection {
  id:
    | 'unverified'
    | 'leaks'
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

/** 所有 hunk 都原样出现在新基线里的算「已确认」；有任何一处对不上的算「拿不准」。 */
function leakTiers(leaks: Leak[]): { sure: Leak[]; unsure: Leak[] } {
  const isSure = (l: Leak) => l.hunks.length > 0 && l.hunks.every((h) => h.verdict === 'present')
  return { sure: leaks.filter(isSure), unsure: leaks.filter((l) => !isSure(l)) }
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
  const drift = r.drift ?? []
  const overlap = r.overlap ?? []
  const overlapItem = (o: OverlapFile): ReportItem => ({ text: o.path, meta: o.commits.join('、') })
  // drift 里的已经在「分支与磁盘对不上」那一节，不重复列
  const differs = overlap.filter((o) => !o.matchesDisk && !drift.includes(o.path))
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
  const { sure, unsure } = leakTiers(r.leaks)
  // 顺序就是展示顺序：先 danger，再 attention，最后 info；info 里与这次跟进最相关的在前
  const candidates: Omit<FollowReportSection, 'count'>[] = [
    {
      id: 'drift',
      title: '分支与磁盘对不上',
      level: 'danger',
      note:
        '「本地改动」里会出现这些文件的修改：磁盘上是 SVN 的版本，重放后的分支里却是别的内容' +
        '（或者把 SVN 已删的文件带了回来）。请逐个核对，磁盘上的版本是对的，就把它提交进分支。' +
        '可能是重放时合错了，也可能是你在磁盘上撤掉了某个 commit 的改动、还没提交这次撤销。',
      groups: [{ label: '', items: plain(drift) }],
    },
    {
      id: 'leaks',
      title: '泄漏告警',
      level: 'danger',
      note:
        `${LEAK_NOTE}。只是提醒，不影响这次跟进；` +
        '带着这些补丁的其他分支不会自动移动，切过去时会提示手动跟进。确认是你有意提交的，就可以不管它。',
      groups: [
        { label: '这些改动已确认在 SVN 上', items: sure.map(leakItem) },
        { label: '拿不准', items: unsure.map(leakItem) },
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
    { id: 'changes', title: '跟进的文件', level: 'info', groups: changeGroups(r) },
    {
      id: 'moved',
      title: '一起移动的分支',
      level: 'info',
      note: '这些分支一起重放到了新基线上，没有动工作区。',
      groups: [{ label: '', items: plain(r.movedBranches) }],
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
    safe: '工作区里的文件没有改动',
    basis:
      r.evidence === 'hooks' ? '判定依据：TortoiseSVN 钩子事件' : '判定依据：作者 + 失焦快照',
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
}

/** 报告里能放回 Local-Only 补丁的文件；旧后端没有这个字段时为空。 */
export function restorablePaths(r: FollowReport): string[] {
  return r.restorable ?? []
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
