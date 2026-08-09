import { describe, expect, it } from 'vitest'
import {
  expandPlaceholders,
  planCustomCommand,
  type CommandContext,
  type CustomCommand,
} from '../../src/engine/customCommands'

const ctx: CommandContext = {
  worktree: 'C:\\Users\\me\\dev\\my repo',
  branch: 'feat/toolbar',
  headHash: '0a6c1863f2b1c4d5e6f708192a3b4c5d6e7f8091',
}

const vscode: CustomCommand = {
  id: 'vscode',
  label: 'VS Code',
  program: 'code',
  args: ['$WORKTREE'],
  cwd: '$WORKTREE',
}

describe('expandPlaceholders', () => {
  it('Windows 反斜杠路径原样代入，不做任何转义', () => {
    expect(expandPlaceholders('$WORKTREE', ctx)).toBe('C:\\Users\\me\\dev\\my repo')
  })

  it('作为子串出现时也要替换', () => {
    expect(expandPlaceholders('$WORKTREE/src', ctx)).toBe('C:\\Users\\me\\dev\\my repo/src')
  })

  it('替换 $BRANCH 与 $HEAD', () => {
    expect(expandPlaceholders('$BRANCH@$HEAD', ctx)).toBe(`${ctx.branch}@${ctx.headHash}`)
  })

  it('路径里的 $& $1 是字面量，不能被当成替换模式', () => {
    // String.replace 的字符串替换会把 $& 解释成「整个匹配」，
    // 必须用函数形式的 replace 才不会被反噬
    const weird: CommandContext = { ...ctx, worktree: 'C:\\repo$&$1x' }
    expect(expandPlaceholders('$WORKTREE', weird)).toBe('C:\\repo$&$1x')
  })

  it('替换结果里的占位符不会被二次展开', () => {
    const nested: CommandContext = { ...ctx, worktree: 'C:\\$BRANCH' }
    expect(expandPlaceholders('$WORKTREE', nested)).toBe('C:\\$BRANCH')
  })

  it('未知占位符原样保留，不变成空串', () => {
    expect(expandPlaceholders('$FOO', ctx)).toBe('$FOO')
  })

  it('$HEADER 不能被当成 $HEAD 加 ER', () => {
    expect(expandPlaceholders('$HEADER', ctx)).toBe('$HEADER')
  })
})

describe('planCustomCommand', () => {
  it('展开 args 与 cwd', () => {
    const plan = planCustomCommand(vscode, ctx)
    expect(plan).toEqual({
      program: 'code',
      args: ['C:\\Users\\me\\dev\\my repo'],
      cwd: 'C:\\Users\\me\\dev\\my repo',
    })
  })

  it('program 本身也支持占位符', () => {
    const plan = planCustomCommand({ ...vscode, program: '$WORKTREE\\run.exe' }, ctx)
    expect(plan?.program).toBe('C:\\Users\\me\\dev\\my repo\\run.exe')
  })

  it('空 args 保持为空数组，不能凭空补出参数', () => {
    // 空参数列表在本项目造成过误伤，这里显式钉住
    expect(planCustomCommand({ ...vscode, args: [] }, ctx)?.args).toEqual([])
  })

  it('cwd 为空串时传 null，让子进程继承默认工作目录', () => {
    expect(planCustomCommand({ ...vscode, cwd: '' }, ctx)?.cwd).toBeNull()
  })

  it('没打开仓库时返回 null —— 按钮据此禁用', () => {
    const empty: CommandContext = { worktree: null, branch: '', headHash: '' }
    expect(planCustomCommand(vscode, empty)).toBeNull()
  })
})
