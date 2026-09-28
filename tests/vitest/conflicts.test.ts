import { describe, expect, it } from 'vitest'
import { conflictsFirst, countConflicts, repoStateBanner } from '../../src/engine/conflicts'
import type { FileChange, FileStatus } from '../../src/engine/types'

const file = (path: string, status: FileStatus): FileChange => ({
  path, status, additions: 0, deletions: 0,
})

describe('conflictsFirst', () => {
  it('冲突文件排到最前，其余保持原顺序', () => {
    const files = [
      file('a.txt', 'modified'),
      file('b.txt', 'conflicted'),
      file('c.txt', 'added'),
      file('d.txt', 'conflicted'),
    ]
    expect(conflictsFirst(files).map((f) => f.path)).toEqual(['b.txt', 'd.txt', 'a.txt', 'c.txt'])
  })

  it('没有冲突时原样返回同一个数组（不触发多余的重渲染）', () => {
    const files = [file('a.txt', 'modified'), file('b.txt', 'added')]
    expect(conflictsFirst(files)).toBe(files)
  })

  it('空列表', () => {
    expect(conflictsFirst([])).toEqual([])
  })

  it('不修改入参', () => {
    const files = [file('a.txt', 'modified'), file('b.txt', 'conflicted')]
    conflictsFirst(files)
    expect(files.map((f) => f.path)).toEqual(['a.txt', 'b.txt'])
  })
})

describe('countConflicts', () => {
  it('只数 conflicted', () => {
    expect(countConflicts([
      file('a.txt', 'conflicted'),
      file('b.txt', 'modified'),
      file('c.txt', 'conflicted'),
    ])).toBe(2)
  })

  it('空列表为 0', () => {
    expect(countConflicts([])).toBe(0)
  })
})

describe('repoStateBanner', () => {
  it('normal 不显示横幅', () => {
    expect(repoStateBanner('normal', 0)).toBeNull()
  })

  it('normal 下即便有冲突标记也不显示（没有进行中的操作可收尾）', () => {
    expect(repoStateBanner('normal', 2)).toBeNull()
  })

  it('进行中且有冲突时带上冲突数', () => {
    expect(repoStateBanner('merge', 3)).toBe('merge in progress · 3 个文件有冲突')
  })

  it('没有冲突时不编造「已处理」——interactive rebase 停在 edit 上本来就没有冲突', () => {
    expect(repoStateBanner('rebase', 0)).toBe('rebase in progress')
  })
})
