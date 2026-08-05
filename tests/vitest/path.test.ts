import { describe, expect, it } from 'vitest'
import { normalizePath, samePath } from '../../src/engine/path'

describe('normalizePath', () => {
  it('把反斜杠统一成正斜杠', () => {
    expect(normalizePath('C:\\Users\\me\\repo')).toBe('c:/users/me/repo')
  })

  it('去掉尾部斜杠', () => {
    expect(normalizePath('C:/repo//')).toBe('c:/repo')
  })
})

describe('samePath', () => {
  it('git 输出的正斜杠路径与 Windows 反斜杠路径视为同一个', () => {
    // 这正是 worktree 高亮失效的原因：git 给正斜杠，dialog/常量给反斜杠
    expect(samePath('C:/Users/me/repo', 'C:\\Users\\me\\repo')).toBe(true)
  })

  it('忽略盘符与路径的大小写差异', () => {
    expect(samePath('c:/users/me/repo', 'C:/Users/Me/Repo')).toBe(true)
  })

  it('忽略尾部斜杠差异', () => {
    expect(samePath('C:/repo/', 'C:/repo')).toBe(true)
  })

  it('不同路径仍然判定为不同', () => {
    expect(samePath('C:/repo', 'C:/repo-wt')).toBe(false)
  })
})
