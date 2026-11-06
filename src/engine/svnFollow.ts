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
  lost: string[]
  discarded: string[]
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
    }
  /** 什么都没动：前置条件不满足，或者准备阶段失败。 */
  | { kind: 'refused'; code: string; message: string }

export type FollowActionId = 'followContinue'

/** 后端 `svn_follow_mark::SvnUserInfo`：「我的 SVN 用户名」的两个来源。 */
export interface SvnUserInfo {
  /** 配置里的 `svnUser`。 */
  configured: string | null
  /** `svn auth` 探测到的。 */
  detected: string | null
}

/** {@link describeFollowReport} 的一节。没发生的事不成节。 */
export interface FollowReportSection {
  id:
    | 'leaks'
    | 'lost'
    | 'uncertain'
    | 'discarded'
    | 'merged'
    | 'changes'
    | 'dropped'
    | 'rewritten'
    | 'snapshot'
    | 'drift'
    | 'moved'
    | 'behind'
  title: string
  /** 要用户留意：说明走红底警告条，这一节排在前面。 */
  warn: boolean
  note?: string
  /** `label` 为空的组不画小标题。 */
  groups: { label: string; items: string[] }[]
}

export interface FollowReportModel {
  tone: NoticeTone
  title: string
  summary: string
  /** 整份报告的前提有问题（这次没向服务器验证过）。 */
  warning?: string
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

function capped(items: string[]): string[] {
  if (items.length <= MAX_REPORT_ITEMS) return items
  return [
    ...items.slice(0, MAX_REPORT_ITEMS),
    `…还有 ${items.length - MAX_REPORT_ITEMS} 个`,
  ]
}

const LEAK_NOTE = '这些本地补丁（Local-Only）可能被提交到了 SVN，请检查'

/** 6.10 的快照内容：报告与冲突框共用的开头。 */
const SNAPSHOT_NOTE =
  '下面这些文件在 commit 里的内容与 SVN 上某个较新的版本逐字节相同（多半是 svn update 之后整包提交进 git 的）。' +
  '如果你是有意停在那个版本，请从原 commit 里取回。'

function leakLine(l: Leak): string {
  return `${l.path}（分支 ${l.branch}：${l.subject}）`
}

function leakSection(leaks: Leak[]): string {
  return `${LEAK_NOTE}：\n${listed(leaks.map(leakLine))}`
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
  const byAuthor = new Map<string, string[]>()
  const anonymous: string[] = []
  for (const c of r.changes) {
    const line = `${c.path}  r${c.rev}`
    if (!c.author) anonymous.push(line)
    else byAuthor.set(c.author, [...(byAuthor.get(c.author) ?? []), line])
  }
  const named = [...byAuthor.keys()].sort().map((a) => ({ label: a, items: byAuthor.get(a)! }))
  return [
    ...named,
    { label: '（没有作者）', items: anonymous },
    { label: '已在 SVN 上删除', items: r.removed },
  ]
}

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
  const candidates: FollowReportSection[] = [
    {
      id: 'drift',
      title: '分支与磁盘对不上',
      warn: true,
      note:
        '这些文件在磁盘上是 SVN 的版本，重放后的分支里却是别的内容（或者把 SVN 已删的文件带了回来），' +
        '「本地改动」里会出现它们的修改。可能是重放时合错了，也可能是你在磁盘上撤掉了某个 commit 的改动、' +
        '还没提交这次撤销。请逐个核对：磁盘上的版本是对的，就把它提交进分支。',
      groups: [{ label: '', items: drift }],
    },
    {
      id: 'leaks',
      title: '泄漏告警',
      warn: true,
      note: `${LEAK_NOTE}。`,
      groups: [{ label: '', items: r.leaks.map(leakLine) }],
    },
    {
      id: 'lost',
      title: '丢失的暂存',
      warn: true,
      note: `这些暂存内容没能保留，已取 SVN 的版本。${pinned}`,
      groups: [{ label: '', items: r.stage.lost }],
    },
    {
      id: 'uncertain',
      title: '拿不准的暂存',
      warn: true,
      note: '这些暂存原样保留：分不清它和你的提交谁先谁后，请确认。不需要的话取消暂存即可。',
      groups: [{ label: '', items: r.stage.uncertain }],
    },
    {
      id: 'discarded',
      title: '丢弃的暂存',
      warn: false,
      note: `这些暂存是已提交版本之前的中间版本，已丢弃。${pinned}`,
      groups: [{ label: '', items: r.stage.discarded }],
    },
    {
      id: 'merged',
      title: '合并的暂存',
      warn: false,
      note: '这些暂存和 SVN 的新内容做了三方合并，结果留在暂存区。',
      groups: [{ label: '', items: merged }],
    },
    { id: 'changes', title: '跟进的文件', warn: false, groups: changeGroups(r) },
    {
      id: 'dropped',
      title: '丢弃的 commit',
      warn: false,
      note: '这些 commit 的内容都已经在 SVN 上，重放之后变空，不再保留。',
      groups: [{ label: '', items: r.dropped.map((c) => c.slice(0, 10)) }],
    },
    {
      id: 'rewritten',
      title: '换成 SVN 版本的路径',
      warn: false,
      note: '这些路径你已经提交到了 SVN，commit 里的版本让位给 SVN 的版本。',
      groups: r.rewritten.map(([c, paths]) => ({ label: c.slice(0, 10), items: paths })),
    },
    {
      id: 'snapshot',
      title: '按基线对齐的快照内容',
      warn: false,
      note: `${SNAPSHOT_NOTE}重放时直接采用了新基线的版本。`,
      groups: snapshot.map(([c, paths]) => ({ label: c.slice(0, 10), items: paths })),
    },
    {
      id: 'moved',
      title: '移动的分支',
      warn: false,
      note: '这些分支一起重放到了新基线上，没有动工作区。',
      groups: [{ label: '', items: r.movedBranches }],
    },
    {
      id: 'behind',
      title: '落后的分支',
      warn: false,
      note: '这些分支没有移动，切过去时会提示手动跟进。',
      groups: [{ label: '', items: r.behindBranches.map(([b, why]) => `${b}：${why}`) }],
    },
  ]
  const sections = candidates
    .map((s) => ({
      ...s,
      groups: s.groups
        .filter((g) => g.items.length > 0)
        .map((g) => ({ ...g, items: capped(g.items) })),
    }))
    .filter((s) => s.groups.length > 0)
  const warned = !r.verified || r.leaks.length > 0 || r.stage.lost.length > 0 || drift.length > 0
  const pending = r.stage.uncertain.length > 0 || r.behindBranches.length > 0 || snapshot.length > 0
  const count = r.changes.length + r.removed.length
  const what = count > 0 ? `跟进了 ${count} 个文件` : '分支已跟上基线'
  return {
    tone: warned || pending ? 'attention' : 'ok',
    title: '跟进完成',
    summary: `${what}，基线最低在 r${r.newFloor}。工作区里的文件没有改动。`,
    warning: r.verified
      ? undefined
      : '这次没能向服务器体检新基线（可能是断网），基线照样发布了。连上之后建议手动体检一次。',
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
          ]
        : [
            `基线已经发布，但把提交 ${o.at.slice(0, 10)} 重放上去时有冲突：\n${listed(o.files)}`,
            '分支和暂存区都没动。这需要人来判断：点「手动跟进」，未提交的改动会先收起来，分支重放到' +
              '新基线上，冲突由你解决，最后再把改动放回来。这一步会写工作区里的文件。',
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
