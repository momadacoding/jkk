import { describe, expect, it } from 'vitest'
import { toErrorCode, toErrorMessage } from '../../src/engine/errors'

describe('toErrorMessage', () => {
  it('展开 Tauri 的 GitError 对象而不是显示 [object Object]', () => {
    const err = { code: 'git_cli', message: "fatal: 'feature' is already used by worktree" }
    expect(toErrorMessage(err)).toBe("[git_cli] fatal: 'feature' is already used by worktree")
  })

  it('没有 code 时只显示 message', () => {
    expect(toErrorMessage({ message: 'boom' })).toBe('boom')
  })

  it('保留 Error 实例的 message', () => {
    expect(toErrorMessage(new Error('plain error'))).toBe('plain error')
  })

  it('直接返回字符串', () => {
    expect(toErrorMessage('just a string')).toBe('just a string')
  })

  it('无法识别的对象退化为 JSON 而非 [object Object]', () => {
    const out = toErrorMessage({ unexpected: 1 })
    expect(out).not.toContain('[object Object]')
    expect(out).toBe('{"unexpected":1}')
  })

  it('处理 null 与 undefined', () => {
    expect(toErrorMessage(null)).toBe('null')
    expect(toErrorMessage(undefined)).toBe('undefined')
  })
})

describe('toErrorCode', () => {
  /**
   * 删 worktree 的确认框靠这个 code 决定要不要把按钮升级成「仍要删除」。
   * 靠匹配 message 文案做这件事改一个字就失效。
   */
  it('取出后端 GitError 的 code', () => {
    expect(toErrorCode({ code: 'worktree_dirty', message: '有未提交的改动' })).toBe('worktree_dirty')
  })

  it('没有 code 的一律返回 null，不要瞎猜', () => {
    expect(toErrorCode({ message: 'boom' })).toBeNull()
    expect(toErrorCode(new Error('plain'))).toBeNull()
    expect(toErrorCode('just a string')).toBeNull()
    expect(toErrorCode(null)).toBeNull()
    expect(toErrorCode(undefined)).toBeNull()
  })

  it('code 不是字符串时不当成 code', () => {
    expect(toErrorCode({ code: 42 })).toBeNull()
  })
})
