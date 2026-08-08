/** commit 消息的两个输入框。 */
export type CommitField = 'subject' | 'description'

/** 一次 Enter 该干什么。`none` 表示交还浏览器默认行为，不要 preventDefault。 */
export type CommitKeyAction = 'commit' | 'focus-description' | 'none'

/**
 * 只取 React.KeyboardEvent 里判断所需的那几个字段，
 * 这样这层逻辑不依赖 DOM，可以直接单测。
 */
export interface CommitKeyEvent {
  key: string
  ctrlKey: boolean
  metaKey: boolean
  shiftKey: boolean
  altKey: boolean
  /** 输入法正在组词。 */
  isComposing?: boolean
}

/**
 * commit 输入框里 Enter 的分派。
 *
 * 约定：subject 里 Enter 往下跳到 description，Ctrl/Cmd+Enter 才是提交。
 * subject 是单行 input，Enter 本来就没有换行可言，与其浪费掉不如拿来接续输入；
 * 而提交是不可撤销的破坏性动作，必须要一个带修饰键的组合，避免打字时误触。
 *
 * `canCommit` 与 Commit 按钮的 disabled 用同一个判据传进来，
 * 免得键盘路径能提交一个按钮不让提交的空 subject。
 */
export function commitKeyAction(
  field: CommitField,
  e: CommitKeyEvent,
  canCommit: boolean,
): CommitKeyAction {
  if (e.key !== 'Enter') return 'none'

  // 中文/日文输入法拿 Enter 上屏候选词，这一下不属于用户在「确认表单」。
  // 不挡住的话，打中文时每次选字都会被弹走或直接提交。
  if (e.isComposing) return 'none'

  if (e.ctrlKey || e.metaKey) return canCommit ? 'commit' : 'none'

  // Shift/Alt+Enter 留给系统默认，不要在 subject 里悄悄跳走
  if (e.shiftKey || e.altKey) return 'none'

  // description 里的裸 Enter 就是换行，别拦
  return field === 'subject' ? 'focus-description' : 'none'
}
