import { describe, expect, it } from 'vitest'
import {
  describeConflictList,
  describeResolveWarning,
  offerAfterOutcome,
  opensConflictList,
  type SvnConflict,
} from '@/engine/svnConflicts'

const base: SvnConflict = {
  path: 'Art/x.prefab',
  text: true,
  props: false,
  tree: false,
  dir: false,
  gitManaged: false,
  resolvable: true,
  mergeable: true,
}

describe('describeConflictList', () => {
  it('可合并的文本冲突给四个按钮，二进制和只有属性的给两个', () => {
    const m = describeConflictList([
      base,
      { ...base, path: 'Art/t.png', mergeable: false },
      { ...base, path: 'Art/p.meta', text: false, props: true, mergeable: false },
    ])
    expect(m.rows.map((r) => [r.path, r.kind, r.actions])).toEqual([
      ['Art/x.prefab', '内容冲突', ['mine', 'theirs', 'merge', 'merged']],
      ['Art/t.png', '内容冲突', ['mine', 'theirs']],
      ['Art/p.meta', '属性冲突', ['mine', 'theirs']],
    ])
    expect(m.refused).toEqual([])
    expect(m.done).toBe(false)
  })

  it('树冲突、目录、git 管的文件单列一组，不给按钮', () => {
    const no = { resolvable: false, mergeable: false }
    const m = describeConflictList([
      { ...base, path: 'Art', text: false, props: true, dir: true, ...no },
      { ...base, path: 'Art/gone', text: false, tree: true, ...no },
      { ...base, path: 'Code/A.cs', gitManaged: true, ...no },
    ])
    expect(m.rows).toEqual([])
    expect(m.refused.map((r) => [r.path, r.kind, r.actions])).toEqual([
      ['Art', '目录属性冲突', []],
      ['Art/gone', '树冲突', []],
      ['Code/A.cs', '内容冲突（git 管的文件）', []],
    ])
    expect(m.refusedNote).toContain('TortoiseSVN')
  })

  it('内容和属性都冲突时说清楚', () => {
    expect(describeConflictList([{ ...base, props: true }]).rows[0].kind).toBe('内容和属性都冲突')
  })

  it('空清单就是都解决了，不再提 Unity', () => {
    const m = describeConflictList([])
    expect(m.done).toBe(true)
    expect(m.intro).toBe('冲突都解决了。')
    expect(m.notes).toEqual([])
  })
})

describe('describeResolveWarning', () => {
  it('说出丢掉的是哪一边、不能撤销', () => {
    expect(describeResolveWarning(base, 'mine')).toContain('服务器这次的改动丢掉')
    expect(describeResolveWarning(base, 'theirs')).toContain('本地的改动丢掉')
    expect(describeResolveWarning({ ...base, text: false, props: true }, 'mine')).toContain('属性')
    expect(describeResolveWarning(base, 'theirs')).toContain('不能撤销')
  })
})

describe('offerAfterOutcome', () => {
  it('流程走完才查；停在 git 冲突上不查；没有结局不查', () => {
    for (const k of ['done', 'aborted', 'released', 'settled', 'stageLost', 'followDrift']) {
      expect(offerAfterOutcome(k)).toBe(true)
    }
    for (const k of ['rebaseConflict', 'stashConflict', 'stashBlocked']) {
      expect(offerAfterOutcome(k)).toBe(false)
    }
    expect(offerAfterOutcome(null)).toBe(false)
  })
})

describe('opensConflictList', () => {
  it('只认「有冲突没解决」的那几种错误码', () => {
    for (const code of [
      'sync_in_update_conflict',
      'sync_in_svn_conflicted',
      'svn_preflight_conflicted',
      'sync_out_svn_conflicted',
    ]) {
      expect(opensConflictList({ code, message: '' })).toBe(true)
    }
    expect(opensConflictList({ code: 'sync_in_update_failed', message: '' })).toBe(false)
    expect(opensConflictList(new Error('x'))).toBe(false)
    expect(opensConflictList(null)).toBe(false)
  })
})
