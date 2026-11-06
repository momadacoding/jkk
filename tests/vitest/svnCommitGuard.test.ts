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

  it('分支落后基线时「本地改动」里混着基线带进来的文件：有暂存就提醒', () => {
    const behind: SyncStatus = { ...inSync, headBehindBase: true }
    const w = commitBeforeFollowWarning(['Code/Mine.cs'], behind)!
    expect(w.message).toContain('落后基线')
    expect(commitBeforeFollowWarning([], behind)).toBeNull()
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
