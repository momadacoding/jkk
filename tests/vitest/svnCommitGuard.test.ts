import { describe, expect, it } from 'vitest'
import { commitBeforeFollowWarning } from '@/engine/svnCommitGuard'
import type { SyncStatus } from '@/engine/svnSync'

const base: SyncStatus['base'] = { floor: 3, newer: 0, gone: 0 }

const inSync: SyncStatus = {
  baselineRev: 3,
  nodeRevs: [3],
  verdict: { kind: 'inSync' },
  base,
  headBehindBase: false,
}

const advanced: SyncStatus = {
  ...inSync,
  nodeRevs: [9],
  verdict: {
    kind: 'externallyAdvanced',
    changed: [{ path: 'Code/A.cs', rev: 9, author: 'li' }],
    added: [{ path: 'Code/New.cs', rev: 9, author: null }],
    removed: ['Code/Gone.cs'],
  },
}

describe('commitBeforeFollowWarning（事故分析 6.6）', () => {
  it('暂存里有 SVN 侧的新内容：提醒先跟进，列出这些文件', () => {
    const w = commitBeforeFollowWarning(['Code/A.cs', 'Code/Mine.cs', 'Code/Gone.cs'], advanced)!
    expect(w.message).toContain('Code/A.cs')
    expect(w.message).toContain('Code/Gone.cs')
    expect(w.message).not.toContain('Code/Mine.cs')
    expect(w.message).toContain('跟进')
    expect(w.confirmLabel).toBe('仍然提交')
    // 11-05 的快照提交就是这么来的：说清后果
    expect(w.warning).toContain('记成你的提交')
  })

  it('暂存里没有 SVN 的新内容：不打扰', () => {
    expect(commitBeforeFollowWarning(['Code/Mine.cs'], advanced)).toBeNull()
    expect(commitBeforeFollowWarning([], advanced)).toBeNull()
  })

  it('分支落后基线：暂存落在基线带进来的文件里才提醒，不是有暂存就弹', () => {
    // review 第 3 条：用应用切到一条老分支时磁盘是它自己的内容，「有暂存就弹」会每次提交都打扰
    const behind: SyncStatus = { ...inSync, headBehindBase: true, behindPaths: ['Code/A.cs'] }
    const w = commitBeforeFollowWarning(['Code/A.cs', 'Code/Mine.cs'], behind)!
    expect(w.message).toContain('Code/A.cs')
    expect(w.message).not.toContain('Code/Mine.cs')
    expect(commitBeforeFollowWarning(['Code/Mine.cs'], behind)).toBeNull()
    // 旧后端没有 behindPaths：不打扰
    expect(commitBeforeFollowWarning(['Code/A.cs'], { ...inSync, headBehindBase: true })).toBeNull()
  })

  it('又 update 了一次、分支同时落后基线：两份清单都看', () => {
    // review 第 2 条：externallyAdvanced 但暂存与 SVN 新改动不相交时，不能跳过落后基线那份
    const both: SyncStatus = { ...advanced, headBehindBase: true, behindPaths: ['Code/Old.cs'] }
    const w = commitBeforeFollowWarning(['Code/Old.cs'], both)!
    expect(w.message).toContain('Code/Old.cs')
  })

  it('路径大小写不同也认得（Windows）', () => {
    expect(commitBeforeFollowWarning(['code/a.CS'], advanced)).not.toBeNull()
  })

  it('不是 SVN 仓库、还没读到状态、已经同步：不打扰', () => {
    expect(commitBeforeFollowWarning(['Code/A.cs'], null)).toBeNull()
    expect(commitBeforeFollowWarning(['Code/A.cs'], inSync)).toBeNull()
  })

  it('路径按正斜杠比，Windows 形态的反斜杠也认得', () => {
    expect(commitBeforeFollowWarning(['Code\\A.cs'], advanced)).not.toBeNull()
  })

  it('清单很长时截断', () => {
    const many = Array.from({ length: 40 }, (_, i) => ({ path: `Code/F${i}.cs`, rev: 9, author: null }))
    const s: SyncStatus = {
      ...advanced,
      verdict: { kind: 'externallyAdvanced', changed: many, added: [], removed: [] },
    }
    const w = commitBeforeFollowWarning(many.map((m) => m.path), s)!
    expect(w.message).toContain('还有')
  })
})
