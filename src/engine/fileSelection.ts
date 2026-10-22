import { conflictsFirst } from './conflicts'
import type { StatusData } from './types'

/** 文件列表的两个区。两边的操作语义不同，选中不能跨区。 */
export type FileSection = 'unstaged' | 'staged'

export interface FileSelection {
  /** 选中所属的区；还没选过任何东西时为 null。 */
  section: FileSection | null
  /** 选中的路径，顺序与列表显示顺序一致。 */
  paths: string[]
  /** Shift 范围选择的起点。Shift 不移动它，所以能连续调整范围。 */
  anchor: string | null
}

export interface ClickModifiers {
  ctrl: boolean
  shift: boolean
  /**
   * 点在已选中的项上时保持整片选中不变。右键菜单要用：
   * 选了五个文件却因为右键把选中缩成一个，是最容易误删的交互。
   */
  keepIfSelected?: boolean
}

export const emptyFileSelection: FileSelection = { section: null, paths: [], anchor: null }

/**
 * 算出一次点击之后的选中状态。
 *
 * 抽成纯函数是为了可测：本仓库没有 jsdom，组件跑不起来，
 * 而这段修饰键逻辑（尤其是 Shift 的范围和顺序）恰恰最容易写错。
 *
 * @param current 当前选中
 * @param section 被点击的行属于哪个区
 * @param files 该区的完整路径列表，顺序即显示顺序
 * @param path 被点击的路径
 */
export function nextFileSelection(
  current: FileSelection,
  section: FileSection,
  files: string[],
  path: string,
  mods: ClickModifiers,
): FileSelection {
  // 跨区就整片重来：Unstaged 的 Stage 和 Staged 的 Unstage 是两回事，
  // 混在一起选出来的动作没法解释
  const sameSection = current.section === section
  const prev = sameSection ? current.paths : []
  const anchor = sameSection ? current.anchor : null

  if (mods.keepIfSelected && prev.includes(path)) {
    return { section, paths: prev, anchor: anchor ?? path }
  }

  if (mods.ctrl) {
    const paths = prev.includes(path) ? prev.filter((p) => p !== path) : [...prev, path]
    return { section, paths: sortByList(paths, files), anchor: path }
  }

  if (mods.shift && anchor) {
    const from = files.indexOf(anchor)
    const to = files.indexOf(path)
    if (from >= 0 && to >= 0) {
      const [lo, hi] = from <= to ? [from, to] : [to, from]
      // 锚点不动，这样可以反复 Shift 调整同一段范围
      return { section, paths: files.slice(lo, hi + 1), anchor }
    }
  }

  return { section, paths: [path], anchor: path }
}

/** 右侧 diff 正在显示的那一项：哪个文件、在哪个区。 */
export interface ActiveChange {
  path: string
  section: FileSection
}

/**
 * 状态刷新后让选中与两个列表对齐。选中是唯一的真相：高亮、Delete 等快捷键、
 * 右侧读哪个区的 diff 都从它来，所以要改的是选中本身，而不是在显示时另算一份。
 *
 * - 选中的都还在：原样返回（同一个对象）
 * - 有的不见了：留下其余的
 * - 全不见了，但有的跑到了另一个区（刚被整份暂存 / 取消暂存）：整批跟过去，按那个区的显示顺序
 * - 两边都没有了（刚提交、刚丢弃）：清空
 *
 * 状态刷新时每次渲染都要跑，文件可能上万个，查找用 Set。
 *
 * @param unstaged Unstaged 区的路径，顺序即显示顺序
 * @param staged Staged 区的路径，顺序即显示顺序
 */
export function reconcileSelection(
  selection: FileSelection,
  unstaged: string[],
  staged: string[],
): FileSelection {
  if (!selection.section || selection.paths.length === 0) return selection
  const list = (s: FileSection) => (s === 'unstaged' ? unstaged : staged)
  const own = new Set(list(selection.section))
  const kept = selection.paths.filter((p) => own.has(p))
  if (kept.length === selection.paths.length) return selection
  const keepAnchor = (paths: string[]) =>
    selection.anchor !== null && paths.includes(selection.anchor) ? selection.anchor : paths[0]
  if (kept.length > 0) return { section: selection.section, paths: kept, anchor: keepAnchor(kept) }
  const other: FileSection = selection.section === 'unstaged' ? 'staged' : 'unstaged'
  const wanted = new Set(selection.paths)
  const moved = list(other).filter((p) => wanted.has(p))
  if (moved.length > 0) return { section: other, paths: moved, anchor: keepAnchor(moved) }
  return emptyFileSelection
}

/**
 * 右侧 diff 显示哪一项。区决定读哪种 diff（第 65 条），所以不能只给文件名：
 * 同一个文件可能两个区都有，两边显示的内容不同。
 *
 * 先按 [`reconcileSelection`] 对齐，再取最后点的那一个；没有选中时取 Unstaged 的第一个，
 * Unstaged 空了取 Staged 的第一个。这个默认项只决定右侧显示什么，不算选中、不高亮。
 */
export function activeChange(
  selection: FileSelection,
  unstaged: string[],
  staged: string[],
): ActiveChange | null {
  const s = reconcileSelection(selection, unstaged, staged)
  const path = s.paths[s.paths.length - 1]
  if (path !== undefined && s.section) return { path, section: s.section }
  if (unstaged.length > 0) return { path: unstaged[0], section: 'unstaged' }
  if (staged.length > 0) return { path: staged[0], section: 'staged' }
  return null
}

/**
 * 两个列表在界面上的顺序（Unstaged 里冲突文件排最前）。`RepoTab` 算该读哪个区、
 * `LocalChangesView` 算显示哪一项，都用这一份，默认项才不会两边算得不一样。
 */
export function changesPaths(status: StatusData): { unstaged: string[], staged: string[] } {
  return {
    unstaged: conflictsFirst(status.unstaged).map((f) => f.path),
    staged: status.staged.map((f) => f.path),
  }
}

/**
 * 「本地改动」右侧该读哪个区的 diff：Unstaged 读暂存区对比工作区（scope `unstaged`），
 * Staged 读 HEAD 对比暂存区（scope `staged`）。两边都空时固定为 Unstaged。
 */
export function changesSection(selection: FileSelection, status: StatusData): FileSection {
  const { unstaged, staged } = changesPaths(status)
  return activeChange(selection, unstaged, staged)?.section ?? 'unstaged'
}

/** 按列表显示顺序排列，保证选中顺序和眼睛看到的一致。 */
function sortByList(paths: string[], files: string[]): string[] {
  return files.filter((f) => paths.includes(f))
}
