import { describe, expect, it } from 'vitest'
import { basename, normalizePath, samePath } from '../../src/engine/path'

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

describe('basename', () => {
  it('取 Windows 反斜杠路径的最后一段', () => {
    expect(basename('C:\\Users\\me\\MyRepo')).toBe('MyRepo')
  })

  it('取正斜杠路径的最后一段', () => {
    expect(basename('/home/me/MyRepo')).toBe('MyRepo')
  })

  it('忽略尾部斜杠', () => {
    expect(basename('C:/Users/me/MyRepo/')).toBe('MyRepo')
  })

  it('保留原始大小写——这是给人看的名字，不是比较用的 key', () => {
    expect(basename('C:/Users/me/MyRepo')).toBe('MyRepo')
  })

  it('没有分隔符时原样返回', () => {
    expect(basename('MyRepo')).toBe('MyRepo')
  })

  it('空串返回空串而不是崩', () => {
    expect(basename('')).toBe('')
  })

  it('只有斜杠时回退到原串', () => {
    expect(basename('/')).toBe('/')
  })
})
