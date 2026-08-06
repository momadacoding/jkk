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

/** 按列表显示顺序排列，保证选中顺序和眼睛看到的一致。 */
function sortByList(paths: string[], files: string[]): string[] {
  return files.filter((f) => paths.includes(f))
}
