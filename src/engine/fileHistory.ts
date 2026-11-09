/**
 * 文件历史 / Blame 的纯逻辑（计划 2026-11-08-file-history-blame）。
 *
 * 数据形状与 `src-tauri/src/engine/file_history.rs` 一一对应；Rust 的 `Option` 到这边是 `null`。
 */
import type { CommitNode, ConflictInfo, FileStatus, StatusData } from './types'
import { baselineMap, findDelivered } from './svnGraph'

/** 工作区里还没提交的行，`git blame` 把它归到这个全 0 的 sha 上。 */
export const UNCOMMITTED = '0'.repeat(40)

export interface FileHistoryEntry {
  id: string
  parents: string[]
  author: string
  /** 作者时间，unix 秒。 */
  time: number
  subject: string
  /** 这次提交里这个文件的路径；改名之前的提交是旧路径。 */
  path: string
  /** 改名 / 复制的来源路径。 */
  oldPath: string | null
  status: FileStatus
}

export interface FileHistory {
  /** 从新到旧。 */
  entries: FileHistoryEntry[]
  truncated: boolean
}

export interface BlamePrevious {
  commit: string
  path: string
}

export interface BlameCommit {
  author: string
  time: number
  summary: string
  /** 这个文件在这一提交之前最后一次被改的那一版；根提交没有。 */
  previous: BlamePrevious | null
}

export interface BlameLine {
  commit: string
  origLine: number
  finalLine: number
  origPath: string
  content: string
}

export interface BlameData {
  lines: BlameLine[]
  commits: Record<string, BlameCommit>
  binary: boolean
  truncated: boolean
  totalLines: number
}

/** Blame 哪一版：`rev` 为 null 是工作区。`path` 是那一版里的路径（改名前后不同）。 */
export interface BlameRev {
  rev: string | null
  path: string
}

export type FileHistoryTab = 'history' | 'blame'

/** 打开文件历史视图时带的东西。 */
export interface FileHistoryTarget {
  /** 打开时的路径（最新那一侧）。 */
  path: string
  /** `git log` 的起点，取并集。 */
  revs: string[]
  /** 打开时选中的提交；null = 第一条。 */
  focus: string | null
  /** 打开时 Blame 看哪一版。 */
  blame: BlameRev
  tab: FileHistoryTab
}

/** 完整的 oid：SHA-1 是 40 位，SHA-256 仓库是 64 位。 */
export function isFullOid(id: string): boolean {
  return /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i.test(id)
}

/** 工作区里还没提交的行（全 0 的 sha，SHA-256 仓库里是 64 个 0）。 */
export function isUncommitted(id: string): boolean {
  return /^0+$/.test(id)
}

export function shortId(id: string): string {
  return id.slice(0, 7)
}

/**
 * 从某个提交里的文件打开（提交详情 Changes / File Tree）。
 *
 * 起点是 HEAD 加这个提交（决策 D2）：只从 HEAD 走，别的分支上的提交打开会是空的；只从这个提交走，
 * 又看不到它之后的改动。
 *
 * svn 节点（`svn:N`）不是 git 提交：有本地基线（`baselineOid`）时按那个基线提交算，删除时 Blame 看前一版
 * 基线（`diffBase`，Changes 页就是拿这两版比的）；都没有时只从 HEAD 走、Blame HEAD（独立 review 第 6 条）。
 *
 * `status` 是这个文件在这个提交里的状态：删除了的话，这个提交里没有这个文件，Blame 看它父提交里那一版。
 */
export function historyTargetForCommit(
  path: string,
  commit: Pick<CommitNode, 'id' | 'baselineOid' | 'diffBase'>,
  status: FileStatus | null,
  tab: FileHistoryTab,
): FileHistoryTarget {
  const svn = !isFullOid(commit.id)
  const id = svn ? commit.baselineOid : commit.id
  if (!id || !isFullOid(id)) {
    return { path, revs: ['HEAD'], focus: null, blame: { rev: 'HEAD', path }, tab }
  }
  const before = svn && commit.diffBase && isFullOid(commit.diffBase) ? commit.diffBase : `${id}^`
  return {
    path,
    revs: ['HEAD', id],
    focus: id,
    blame: { rev: status === 'deleted' ? before : id, path },
    tab,
  }
}

/**
 * 从 Local Changes 打开。`blameRev` 来自 `workdirHistoryInfo`：null 是 Blame 工作区（没提交的行单独标出来），
 * `'HEAD'` 是工作区里已经没有这个文件、退回 HEAD 那一版。
 */
export function historyTargetForWorkdir(
  path: string,
  blameRev: string | null,
  tab: FileHistoryTab,
): FileHistoryTarget {
  return { path, revs: ['HEAD'], focus: null, blame: { rev: blameRev, path }, tab }
}

export interface WorkdirHistoryInfo {
  /** 非空时历史 / Blame 两项置灰，并在标签后写明原因。 */
  disabledReason: string | null
  /** Blame 看哪一版：null = 工作区，`'HEAD'` = 工作区里没有这个文件了。 */
  blameRev: string | null
}

/**
 * Local Changes 里右键一个文件：能不能看历史 / Blame，Blame 看哪一版。
 *
 * 两个区一起看，不只看右键点的那个区（/code-review 第 1 条）——同一个文件可能两边各有一条：
 *
 * - **HEAD 里有没有它**（没有的话 `git blame` 报「no such path in HEAD」，历史也是空的）：
 *   暂存区里有它就按暂存区那条判（新增 / 改名 / 复制的新路径 = 没有；`git rm --cached` 之后暂存区是删除、
 *   工作区是未跟踪，HEAD 里照样有）；否则按工作区那条（未跟踪 = 没有）。冲突文件看 `ours`（stage 2，
 *   恒为 HEAD 那一侧）：`AA` / `DU` 这类 HEAD 里没有；冲突信息还没读到时先放行，blame 失败会如实报错。
 * - **工作区里有没有它**：工作区那条是删除，或者只有暂存区一条删除（`git rm`），就没有，Blame 退回 HEAD。
 */
export function workdirHistoryInfo(
  path: string,
  status: StatusData,
  conflicts: ConflictInfo[] | null,
): WorkdirHistoryInfo {
  const staged = status.staged.find((f) => f.path === path)
  const unstaged = status.unstaged.find((f) => f.path === path)
  const notInHead: FileStatus[] = ['added', 'renamed', 'copied']
  let inHead: boolean
  if (staged) inHead = !notInHead.includes(staged.status)
  else if (unstaged?.status === 'conflicted') {
    const info = conflicts?.find((c) => c.path === path)
    inHead = info ? info.ours !== null : true
  } else inHead = unstaged ? unstaged.status !== 'added' : true
  const inWorktree = unstaged
    ? unstaged.status !== 'deleted'
    : staged?.status !== 'deleted'
  return {
    disabledReason: inHead ? null : '新文件还没有历史',
    blameRev: inWorktree ? null : 'HEAD',
  }
}

/** 初始选中：focus 在列表里就选它，否则第一条。 */
export function initialEntryId(entries: FileHistoryEntry[], focus: string | null): string | null {
  if (focus && entries.some((e) => e.id === focus)) return focus
  return entries[0]?.id ?? null
}

/** 历史里某一条该 Blame 哪一版。删除那一条里没有这个文件，看父提交里的。 */
export function blameRevForEntry(entry: FileHistoryEntry): BlameRev | null {
  if (entry.status === 'deleted') {
    const parent = entry.parents[0]
    return parent ? { rev: parent, path: entry.path } : null
  }
  return { rev: entry.id, path: entry.path }
}

/** 「Blame 此改动之前的版本」：git 给的 previous；没有（根提交）时 null。 */
export function previousBlameRev(info: BlameCommit | undefined): BlameRev | null {
  if (!info?.previous) return null
  return { rev: info.previous.commit, path: info.previous.path }
}

export interface BlameBlock {
  commit: string
  /** 在 `lines` 里的下标。 */
  start: number
  length: number
}

/** 同一提交连续的行合成一块：块首行显示提交信息，其余行留空，一眼看出改动的范围。 */
export function blameBlocks(lines: BlameLine[]): BlameBlock[] {
  const blocks: BlameBlock[] = []
  lines.forEach((line, i) => {
    const last = blocks[blocks.length - 1]
    if (last && last.commit === line.commit) last.length += 1
    else blocks.push({ commit: line.commit, start: i, length: 1 })
  })
  return blocks
}

/**
 * 每个提交的「新旧」，0 = 最旧，1 = 最新；左侧色条按它着色。
 *
 * 按时间**排名**而不是按时间线性插值：一个十年的仓库里，按时间算的话近两年的提交全挤在一种颜色里。
 * 同一时间的提交名次相同。未提交的行一律算最新。
 */
export function blameHeat(commits: Record<string, BlameCommit>): Map<string, number> {
  const ids = Object.keys(commits)
  const hasUncommitted = ids.some(isUncommitted)
  const times = [...new Set(ids.filter((id) => !isUncommitted(id)).map((id) => commits[id].time))]
    .sort((a, b) => a - b)
  const ranks = times.length + (hasUncommitted ? 1 : 0)
  const scale = (rank: number) => (ranks <= 1 ? 1 : rank / (ranks - 1))
  // 名次表建一次：在循环里 indexOf 是「提交数 × 时间数」，几千个提交的 blame 每次加载都要几百万次比较
  const rankOf = new Map(times.map((t, i) => [t, i]))
  const heat = new Map<string, number>()
  for (const id of ids) {
    heat.set(id, isUncommitted(id) ? 1 : scale(rankOf.get(commits[id].time) ?? 0))
  }
  return heat
}

/**
 * 「在提交图中定位」该选中哪个 id；不在图上时 null（调用方说明原因，不留悬空的选中）。
 *
 * - 混合仓库（传 `gitCommits`）里基线提交被接合成了 `svn:N` 节点，换成那个节点的 id
 * - 交付过的本地提交收在 svn 节点里、不在图上，但选中它照样能显示（详情显示它自己，图定位到收着它的节点）
 */
export function graphSelectionFor(
  id: string,
  commits: CommitNode[],
  gitCommits: CommitNode[] | null,
): string | null {
  const graphId = gitCommits ? baselineMap(gitCommits).get(id) ?? id : id
  if (commits.some((c) => c.id === graphId)) return graphId
  if (findDelivered(commits, graphId)) return graphId
  return null
}

/** Blame 页顶上说明现在看的是哪一版。 */
export function describeBlameRev(target: BlameRev, entries: FileHistoryEntry[]): string {
  if (target.rev === null) return '工作区（含未提交的改动）'
  const rev = target.rev
  if (rev.endsWith('^') && isFullOid(rev.slice(0, -1))) return `${shortId(rev)} 之前的版本`
  if (!isFullOid(rev)) return rev
  const hit = entries.find((e) => e.id === rev)
  return hit ? `${shortId(rev)} ${hit.subject}` : shortId(rev)
}
