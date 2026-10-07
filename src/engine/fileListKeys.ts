import type { FileSection, FileSelection } from './fileSelection'

/** 只取判断所需的字段，这层逻辑不依赖 DOM，可以直接单测。 */
export interface FileListKeyEvent {
  key: string
  ctrlKey: boolean
  metaKey: boolean
  shiftKey: boolean
  altKey: boolean
  /** 按住不放产生的自动重复。 */
  repeat?: boolean
  /** 输入法正在组词。 */
  isComposing?: boolean
}

/** 与右键菜单 `Discard changes…` 完全相同的入参。 */
export interface DiscardRequest {
  paths: string[]
  staged: boolean
}

/**
 * 文件列表里按 Delete 要丢弃哪些文件；`null` 表示这一下不归我们管。
 *
 * 只处理**有焦点的那个区**自己的选中：焦点在 Unstaged（比如 Tab 进来的）而选中
 * 在 Staged 时，按下去丢的是用户眼前没在看的那一边。
 *
 * 只给出目标，确认框照旧由 `onDiscard` 弹——快捷键只是右键菜单的另一个入口，
 * 不能因为是键盘触发就跳过确认。
 *
 * 目标只取 selection，**不**退回到 diff 面板默认显示的那个文件：没点过任何行
 * 就按 Delete，大概率是焦点不在用户以为的地方，丢弃一个没被选中的文件不可原谅。
 *
 * @param focused 收到这次按键的列表属于哪个区
 * @param files 两个区当前的路径列表。selection 可能是刷新前留下的，
 *   已经不在该区的路径（被外部 stage 掉、已经被丢弃）要剔掉。
 */
export function discardShortcut(
  e: FileListKeyEvent,
  focused: FileSection,
  selection: FileSelection,
  files: Record<FileSection, string[]>,
): DiscardRequest | null {
  if (e.key !== 'Delete') return null
  // Shift+Delete 之类的组合留着，以后可能另有含义，别先占了
  if (e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return null
  // 按住不放时每一下重复都是一次丢弃请求，确认框关掉后会接着弹
  if (e.repeat || e.isComposing) return null

  if (selection.section !== focused) return null
  const present = files[focused]
  const paths = selection.paths.filter((p) => present.includes(p))
  if (paths.length === 0) return null
  return { paths, staged: focused === 'staged' }
}
