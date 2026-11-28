/**
 * SVN 冲突列表的纯逻辑（设计 2026-11-28-update-conflict-resolve-in-jkk §3.2）。
 *
 * 后端给的是工作副本里所有的 SVN 冲突（`svn_update_conflicts`）；这里决定怎么分组、每一行给哪些按钮、
 * 点「用我的」「用服务器的」之前怎么提醒。不依赖 React，也不依赖 Tauri。
 */
import { toErrorCode } from './errors'

/** 一处 SVN 冲突，与 Rust 侧 `svn_conflict::SvnConflict` 一一对应。 */
export interface SvnConflict {
  /** 相对工作副本根，正斜杠。 */
  path: string
  text: boolean
  props: boolean
  tree: boolean
  dir: boolean
  gitManaged: boolean
  /** jkk 能放行、能解决：不是树冲突、不是目录、不是 git 管的文件。 */
  resolvable: boolean
  /** 能交给外部合并工具：有内容冲突、不是二进制。 */
  mergeable: boolean
}

/** 与 Rust 侧 `ResolveChoice` 一一对应。 */
export type ResolveChoice = 'mine' | 'theirs' | 'merged'

/** 一行上的按钮。`merge` 打开外部合并工具，其余三种直接发 `svn_resolve_conflict`。 */
export type ConflictAction = 'mine' | 'theirs' | 'merge' | 'merged'

export interface ConflictRow {
  path: string
  /** 「内容冲突」「属性冲突」「内容和属性都冲突」「树冲突」…… */
  kind: string
  actions: ConflictAction[]
}

export interface ConflictListModel {
  title: string
  intro: string
  /** jkk 里能解决的。 */
  rows: ConflictRow[]
  /** 要去 TortoiseSVN 的。不给按钮。 */
  refused: ConflictRow[]
  refusedNote: string
  /** 一个冲突都不剩了。 */
  done: boolean
  notes: string[]
}

/**
 * 这些错误码说的是「工作副本里有 SVN 冲突没解决」：报错之外再弹一次冲突列表（设计 §3.2）。
 * `sync_in_update_conflict` 是更新因为不能放行的冲突整次回退了：列表把要去 TortoiseSVN 的和能在这里解决的分开给人看。
 */
export const CONFLICT_LIST_CODES: ReadonlySet<string> = new Set([
  'sync_in_update_conflict',
  'sync_in_svn_conflicted',
  'svn_preflight_conflicted',
  'sync_out_svn_conflicted',
  // 交付提交之后 D6 对白名单 update 撞上冲突：其中 git 不管的文件同样能在列表里解决（独立 review）
  'sync_out_update_conflict',
])

/**
 * 更新（或横幅上的继续 / 放弃）走到这个结局之后要不要查一次冲突列表。停在 git 冲突上（rebase、放回改动）
 * 时不查：流程还没走完，用户正在解 git 的冲突，等点「继续」走完再查。
 */
export function offerAfterOutcome(kind: string | null): boolean {
  return kind !== null && !['rebaseConflict', 'stashConflict', 'stashBlocked'].includes(kind)
}

export function opensConflictList(err: unknown): boolean {
  const code = toErrorCode(err)
  return code !== null && CONFLICT_LIST_CODES.has(code)
}

export const ACTION_LABELS: Record<ConflictAction, string> = {
  mine: '用我的',
  theirs: '用服务器的',
  merge: '用外部工具合并',
  merged: '标记已解决',
}

function kindOf(c: SvnConflict): string {
  if (c.tree) return '树冲突'
  if (c.dir) return c.props ? '目录属性冲突' : '目录冲突'
  const what = c.text && c.props ? '内容和属性都冲突' : c.props ? '属性冲突' : '内容冲突'
  return c.gitManaged ? `${what}（git 管的文件）` : what
}

function actionsOf(c: SvnConflict): ConflictAction[] {
  if (!c.resolvable) return []
  const out: ConflictAction[] = ['mine', 'theirs']
  // 外部工具合并之后用户自己点「标记已解决」：jkk 不知道工具里存没存
  if (c.mergeable) out.push('merge', 'merged')
  return out
}

const row = (c: SvnConflict): ConflictRow => ({ path: c.path, kind: kindOf(c), actions: actionsOf(c) })

export function describeConflictList(conflicts: SvnConflict[]): ConflictListModel {
  const rows = conflicts.filter((c) => c.resolvable).map(row)
  const refused = conflicts.filter((c) => !c.resolvable).map(row)
  const done = conflicts.length === 0
  // 只剩要去 TortoiseSVN 的（更新整次回退之后常见）：不能说「这些文件 git 不管」，下面列的正是 git 管的和树冲突
  const intro = done
    ? '冲突都解决了。'
    : rows.length === 0
      ? '工作副本里还有 SVN 冲突，下面这些本应用不处理，请用 TortoiseSVN 解决。解决之后才能再「更新」或「交付」。'
      : '下面这些文件 git 不管，SVN 更新时与服务器的版本冲突了。解决之后才能再「更新」或「交付」。'
  return {
    title: 'SVN 冲突',
    intro,
    rows,
    refused,
    refusedNote:
      '下面这些是树冲突、目录上的冲突或 git 管的文件，本应用不处理，请用 TortoiseSVN 解决（Resolve）。',
    done,
    notes: done
      ? []
      : ['Unity 开着时，带冲突标记的文本文件（.prefab、.unity 等）导入会报错，与用 TortoiseSVN 时相同，解决后会恢复。'],
  }
}

/** 点了要先在行里确认的操作。 */
export type ConfirmChoice = 'mine' | 'theirs' | 'merged'

/**
 * 这个操作要不要先确认：「用我的」「用服务器的」都会改写文件，一律确认；「标记已解决」只在同时有属性冲突时
 * 确认——`--accept working` 会把属性冲突一并按本地的值解决，服务器这次的属性改动丢掉（独立 review）。
 */
export function needsConfirm(c: SvnConflict, action: ConflictAction): action is ConfirmChoice {
  return action === 'mine' || action === 'theirs' || (action === 'merged' && c.props)
}

/** 确认时的提醒。 */
export function describeResolveWarning(c: SvnConflict, choice: ConfirmChoice): string {
  const what = c.text && c.props ? '内容和属性' : c.props ? '属性' : '内容'
  if (choice === 'merged') {
    return '内容用你在外部工具里保存的结果；属性冲突一并按你本地的值解决，服务器这次的属性改动丢掉。不能撤销。'
  }
  return choice === 'mine'
    ? `整个文件的${what}用你更新前的版本，服务器这次的改动丢掉。不能撤销。`
    : `整个文件的${what}换成服务器的版本，你本地的改动丢掉。不能撤销。`
}

/** 每种操作执行中按钮上的字。 */
export const BUSY_LABELS: Record<ConflictAction, string> = {
  mine: '正在处理…',
  theirs: '正在处理…',
  merge: '正在打开…',
  merged: '正在处理…',
}
