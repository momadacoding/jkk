/** 一条自定义命令，与 Rust 侧 `CustomCommand` 一一对应。 */
export interface CustomCommand {
  id: string
  label: string
  program: string
  args: string[]
  cwd: string
}

/** 占位符的取值来源，由当前打开的仓库决定。 */
export interface CommandContext {
  /** 当前 worktree 绝对路径；未打开仓库时为 null。 */
  worktree: string | null
  branch: string
  headHash: string
}

/** 展开后可以直接交给后端的一次调用。 */
export interface CustomCommandPlan {
  program: string
  args: string[]
  cwd: string | null
}

/**
 * `\b` 是必须的：没有它，`$HEADER` 会被吃掉前半截变成 `<hash>ER`。
 * 未列出的名字不匹配，因而原样保留。
 */
const PLACEHOLDER = /\$(WORKTREE|BRANCH|HEAD)\b/g

/**
 * 把一段文本里的占位符替换成当前仓库的值。
 *
 * 用函数形式的 replace 而不是字符串形式：字符串替换里 `$&`、`$1` 有特殊含义，
 * 而仓库路径是用户给的、什么字符都可能有。同时整段只扫一遍，
 * 替换进去的值不会被再次展开。
 */
export function expandPlaceholders(text: string, ctx: CommandContext): string {
  return text.replace(PLACEHOLDER, (match, name: string) => {
    switch (name) {
      case 'WORKTREE':
        return ctx.worktree ?? match
      case 'BRANCH':
        return ctx.branch
      case 'HEAD':
        return ctx.headHash
      default:
        return match
    }
  })
}

/**
 * 把一条配置翻译成一次具体的后端调用。
 *
 * 没打开仓库时返回 null——所有默认命令都要用 `$WORKTREE`，
 * 拿不到路径就不该让按钮可点。
 */
export function planCustomCommand(
  cmd: CustomCommand,
  ctx: CommandContext,
): CustomCommandPlan | null {
  if (ctx.worktree === null) return null
  const cwd = expandPlaceholders(cmd.cwd, ctx)
  return {
    program: expandPlaceholders(cmd.program, ctx),
    args: cmd.args.map((arg) => expandPlaceholders(arg, ctx)),
    // 空串会被 Rust 当成一个真实目录去 chdir，必然失败；显式传 null 表示「不设」
    cwd: cwd === '' ? null : cwd,
  }
}
