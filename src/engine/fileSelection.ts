import { buildFileTree, treeFilePaths, type TreeNode } from './fileTree'
import type { FileChange, StatusData } from './types'

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

  if (mods.shift && anchor) return rangeSelection(section, files, anchor, path)

  return { section, paths: [path], anchor: path }
}

/**
 * 从锚点到 `path` 的一段。Shift+单击和按住拖动共用。
 *
 * 锚点不动，这样可以反复 Shift 调整、或拖回来缩小同一段范围。锚点不在列表里
 * （拖到一半状态刷新、它被暂存走了）时退回到只选 `path`，并以它为新锚点。
 *
 * @param files 该区的完整路径列表，顺序即显示顺序
 */
export function rangeSelection(
  section: FileSection,
  files: string[],
  anchor: string,
  path: string,
): FileSelection {
  const from = files.indexOf(anchor)
  const to = files.indexOf(path)
  if (from < 0 || to < 0) return { section, paths: [path], anchor: path }
  const [lo, hi] = from <= to ? [from, to] : [to, from]
  return { section, paths: files.slice(lo, hi + 1), anchor }
}

/**
 * Ctrl+A：选中该区全部文件。
 *
 * 同区的锚点还在就保留，之后 Shift+单击仍从用户原来点的那一行算；否则取第一个。
 * 本区为空时原样返回——在空列表里按 Ctrl+A 不该把另一个区的选中清掉。
 *
 * @param files 该区的完整路径列表，顺序即显示顺序
 */
export function selectAllFiles(
  current: FileSelection,
  section: FileSection,
  files: string[],
): FileSelection {
  if (files.length === 0) return current
  const anchor = current.section === section && current.anchor !== null && files.includes(current.anchor)
    ? current.anchor
    : files[0]
  return { section, paths: [...files], anchor }
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
 * 两个列表在界面上的顺序：按目录树显示的顺序（目录在前、同类按名字，含冲突的排最前，
 * 见 `buildFileTree`）。`RepoTab` 算该读哪个区、`LocalChangesView` 算显示哪一项，
 * 都用这一份，默认项才不会两边算得不一样。
 *
 * 这里是**全部**文件、不管目录收没收起：收起只是 `LocalChangesView` 里的显示状态，
 * Ctrl+A、对齐选中、默认项都不该随它变。
 */
export function changesPaths(status: StatusData): { unstaged: string[], staged: string[] } {
  return {
    unstaged: treeFilePaths(changesTree(status.unstaged)),
    staged: treeFilePaths(changesTree(status.staged)),
  }
}

/**
 * 按列表对象缓存的树。一次状态刷新里 RepoTab 对齐选中、算 diff scope，
 * LocalChangesView 算默认项、画两个区，都要这棵树；改动可能上万条，不能每处各建一遍。
 * 状态对象是整份替换的，同一个数组引用内容不会变，按引用缓存是安全的。
 */
const treeCache = new WeakMap<FileChange[], TreeNode[]>()

/**
 * 本地改动一个区的目录树。`changesPaths` 和 `LocalChangesView` 画树共用这一棵，
 * 显示顺序与选中、默认项用的顺序才不会分叉。
 *
 * 文件节点的 `path` 换回状态里的原样路径：`buildFileTree` 会把反斜杠统一成正斜杠，
 * 而选中、stage / discard、冲突查找都要拿 git 报上来的那个写法去对。
 * 返回的树是共享的，调用方不要改它。
 */
export function changesTree(files: FileChange[]): TreeNode[] {
  const cached = treeCache.get(files)
  if (cached) return cached
  const original = new Map(files.map((f) => [f.path.replace(/\\/g, '/'), f.path]))
  const tree = buildFileTree(files.map((f) => ({ path: f.path, status: f.status })))
  const restore = (nodes: TreeNode[]) => {
    for (const node of nodes) {
      if (node.kind === 'dir') restore(node.children)
      else node.path = original.get(node.path) ?? node.path
    }
  }
  restore(tree)
  treeCache.set(files, tree)
  return tree
}

/**
 * 「本地改动」右侧要读的 diff scope：只读显示的那一个文件，`<区>:<路径>`。
 *
 * 按选中文件所在的区读（第 65 条）：Unstaged 读暂存区对比工作区，Staged 读 HEAD 对比暂存区。
 * 只读一个文件（2026-11-05 排查报告 6.13 方案 C）：右侧本来就只显示这一个，整区读要把整个工作区
 * stat 一遍、再读出每个改动文件的内容。两边都空时为 `null`，什么都不读。
 */
export function changesDiffScope(selection: FileSelection, status: StatusData): string | null {
  const { unstaged, staged } = changesPaths(status)
  const active = activeChange(selection, unstaged, staged)
  return active ? `${active.section}:${active.path}` : null
}

/** 是不是 `changesDiffScope` 给出的单文件 scope。提交详情的 `commit:` / `range:` 不算。 */
export function isChangesFileScope(scope: string): boolean {
  return /^(staged|unstaged):./.test(scope)
}

/** 按列表显示顺序排列，保证选中顺序和眼睛看到的一致。 */
function sortByList(paths: string[], files: string[]): string[] {
  return files.filter((f) => paths.includes(f))
}
