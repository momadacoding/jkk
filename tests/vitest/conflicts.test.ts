import { describe, expect, it } from 'vitest'
import { conflictsFirst, countConflicts, describeStageConflicts, repoStateBanner } from '../../src/engine/conflicts'
import type { ConflictInfo, FileChange, FileStatus } from '../../src/engine/types'

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

describe('describeStageConflicts', () => {
  const STAGE = { oid: 'a'.repeat(40), mode: 0o100644 }
  const info = (path: string, over: Partial<ConflictInfo> = {}): ConflictInfo => ({
    path, code: 'UU', base: STAGE, ours: STAGE, theirs: STAGE,
    case: 'markers', reason: null, driver: null, sameAsHead: false, ...over,
  })
  const unstaged = [
    file('c.txt', 'conflicted'),
    file('clean.txt', 'conflicted'),
    file('img.png', 'conflicted'),
    file('du.txt', 'conflicted'),
    file('a.txt', 'modified'),
  ]
  const infos = [
    info('c.txt'),
    info('clean.txt', { case: 'noMarkers' }),
    info('img.png', { case: 'notTextMerged', reason: 'binaryContent' }),
    info('du.txt', { case: 'deleteModify', code: 'DU', ours: null }),
  ]

  it('普通文件不拦', () => {
    expect(describeStageConflicts(['a.txt'], unstaged, infos)).toBeNull()
  })

  it('没有冲突标记、二进制冲突、一边删一边改都不拦', () => {
    expect(describeStageConflicts(['clean.txt', 'img.png', 'du.txt', 'a.txt'], unstaged, infos)).toBeNull()
  })

  it('有冲突标记的文件要确认，默认按钮是取消，文件名写进标题', () => {
    const req = describeStageConflicts(['c.txt'], unstaged, infos)
    expect(req).not.toBeNull()
    expect(req?.title).toContain('c.txt')
    expect(req?.focusCancel).toBe(true)
  })

  it('混着普通文件：清单只列有标记的那个，并说明取消后整批都不暂存', () => {
    const req = describeStageConflicts(['a.txt', 'c.txt', 'clean.txt'], unstaged, infos)
    expect(req?.message).toContain('c.txt')
    expect(req?.message).not.toContain('clean.txt')
    expect(req?.message).toMatch(/3 个文件都不暂存/)
  })

  it('冲突信息还没读到（null）：冲突文件一律拦，不当成「没有标记」', () => {
    const req = describeStageConflicts(['clean.txt'], unstaged, null)
    expect(req?.message).toContain('clean.txt')
    expect(req?.message).toMatch(/还没读到/)
  })

  it('读到的冲突信息里没有这个文件（status 比冲突信息新）：同样按没读到拦', () => {
    const req = describeStageConflicts(['clean.txt'], unstaged, [])
    expect(req?.message).toContain('clean.txt')
    expect(req?.message).toMatch(/还没读到/)
  })

  it('冲突信息没读到，但要暂存的都是普通文件：不拦', () => {
    expect(describeStageConflicts(['a.txt'], unstaged, null)).toBeNull()
  })

  it('空路径列表不拦', () => {
    expect(describeStageConflicts([], unstaged, null)).toBeNull()
  })

  it('多个文件有标记：标题写个数，清单逐个列出', () => {
    const more = [...unstaged, file('d.txt', 'conflicted')]
    const req = describeStageConflicts(['c.txt', 'd.txt'], more, [...infos, info('d.txt')])
    expect(req?.title).toContain('2 个文件')
    expect(req?.message).toContain('c.txt')
    expect(req?.message).toContain('d.txt')
  })

  it('有标记的和没读到的同时存在：两组分开列', () => {
    const more = [...unstaged, file('e.txt', 'conflicted')]
    const req = describeStageConflicts(['c.txt', 'e.txt'], more, infos)
    expect(req?.message).toMatch(/还有冲突标记[^]*c\.txt/)
    expect(req?.message).toMatch(/还没读到[^]*e\.txt/)
  })
})
