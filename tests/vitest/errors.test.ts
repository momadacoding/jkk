import { describe, expect, it } from 'vitest'
import { toErrorMessage } from '../../src/engine/errors'

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
