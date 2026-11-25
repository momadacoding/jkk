import { describe, expect, it } from 'vitest'
import {
  describeBaselineMismatch,
  manualCheckBlockedReason,
  mismatchCheckApplies,
  mismatchKey,
  type BaselineMismatch,
} from '@/engine/svnMismatch'
import type { SyncState } from '@/engine/svnJournal'
import type { EolStatus, SyncStatus, SyncVerdict } from '@/engine/svnSync'

function st(verdict: SyncVerdict, headBehindBase = false): SyncStatus {
  return {
    baselineRev: 4,
    nodeRevs: [4],
    verdict,
    base: { floor: 4, newer: 0, gone: 0 },
    headBehindBase,
  }
}

const inSync = st({ kind: 'inSync' })
const advanced = st({
  kind: 'externallyAdvanced',
  changed: [{ path: 'Code/A.cs', rev: 5, author: 'x' }],
  added: [],
  removed: [],
})
const diverged = st({ kind: 'diverged', nodeRev: 3, baselineRev: 4 })
const eolDone: EolStatus = { ruleInstalled: true, needsMigration: false }
const eolNeeds: EolStatus = { ruleInstalled: false, needsMigration: true }
const clean: SyncState = { locked: false, holder: null, unfinished: null }
const locked: SyncState = {
  locked: true,
  holder: { opid: 'sync-1', pid: 1, startedAt: 0 },
  unfinished: null,
} as unknown as SyncState

const hit = (files: string[], extra: Partial<BaselineMismatch> = {}): BaselineMismatch => ({
  files,
  total: files.length,
  truncated: false,
  ...extra,
})

describe('mismatchCheckApplies', () => {
  it('同步、分支不落后、行尾已迁移、没有进行中的同步时才判定', () => {
    expect(mismatchCheckApplies(true, inSync, eolDone, clean)).toBe(true)
    expect(mismatchCheckApplies(true, inSync, eolDone, null)).toBe(true)
  })

  it('普通 git 仓库不判定', () => {
    expect(mismatchCheckApplies(false, inSync, eolDone, clean)).toBe(false)
  })

  it('还没读到失同步结论时不判定：打开前 TortoiseSVN 可能已经 update 过', () => {
    expect(mismatchCheckApplies(true, null, eolDone, clean)).toBe(false)
  })

  it('SVN 侧有新内容时不判定：刚 update 下来的文件也是「git 改了、SVN 没改」', () => {
    expect(mismatchCheckApplies(true, advanced, eolDone, clean)).toBe(false)
  })

  it('分支落后基线时不判定：重建基线之后就是这个状态，由「跟进」接手', () => {
    expect(mismatchCheckApplies(true, st({ kind: 'inSync' }, true), eolDone, clean)).toBe(false)
  })

  it('回退等失同步状态不判定：顶部已经有对应横幅', () => {
    expect(mismatchCheckApplies(true, diverged, eolDone, clean)).toBe(false)
  })

  it('行尾迁移没做、或行尾状态还没读到时不判定', () => {
    expect(mismatchCheckApplies(true, inSync, eolNeeds, clean)).toBe(false)
    expect(mismatchCheckApplies(true, inSync, null, clean)).toBe(false)
  })

  it('同步进行中不判定：基线可能停在中间态', () => {
    expect(mismatchCheckApplies(true, inSync, eolDone, locked)).toBe(false)
  })
})

describe('describeBaselineMismatch', () => {
  it('没有命中时不挂', () => {
    expect(describeBaselineMismatch(null, null)).toBeNull()
    expect(describeBaselineMismatch(hit([]), null)).toBeNull()
  })

  it('第一句是要用户做的事，按钮是「体检」', () => {
    const m = describeBaselineMismatch(hit(['Code/A.cs', 'Code/B.cs']), null)!
    expect(m.title.startsWith('点「体检」')).toBe(true)
    expect(m.title).toContain('2 个文件')
    expect(m.actions).toEqual([{ id: 'check', label: '体检' }])
    expect(m.files).toEqual(['Code/A.cs', 'Code/B.cs'])
    expect(m.tone).toBe('warn')
  })

  it('说清楚为什么觉得不对：TortoiseSVN 说没改、分支没碰过、jkk 却显示改了', () => {
    const m = describeBaselineMismatch(hit(['Code/A.cs']), null)!
    expect(m.detail).toContain('TortoiseSVN 显示这些文件没有修改')
    expect(m.detail).toContain('分支提交也没碰过')
    expect(m.detail).toContain('本地改动')
  })

  it('只列了一部分时说出来', () => {
    const m = describeBaselineMismatch(hit(['Code/A.cs'], { total: 80 }), null)!
    expect(m.title).toContain('80 个文件')
    expect(m.detail).toContain('只列出前 1 个')
  })

  it('候选只查了一部分时数目说成「至少」', () => {
    const m = describeBaselineMismatch(hit(['Code/A.cs'], { truncated: true }), null)!
    expect(m.title).toContain('至少 1 个文件')
  })

  it('同一组文件体检过没问题之后不再挂，文件变了再挂', () => {
    const files = ['Code/A.cs']
    expect(describeBaselineMismatch(hit(files), mismatchKey(files))).toBeNull()
    expect(describeBaselineMismatch(hit(['Code/A.cs', 'Code/C.cs']), mismatchKey(files))).not.toBeNull()
  })
})

describe('manualCheckBlockedReason', () => {
  it('正常时可点，回退等失同步状态也可点（那几条横幅自己也给体检）', () => {
    expect(manualCheckBlockedReason(inSync, eolDone, false, clean)).toBeNull()
    expect(manualCheckBlockedReason(null, null, false, null)).toBeNull()
    expect(manualCheckBlockedReason(diverged, eolDone, false, clean)).toBeNull()
  })

  it('同步进行中置灰', () => {
    expect(manualCheckBlockedReason(inSync, eolDone, false, locked)).toContain('同步')
  })

  it('基线丢了且还没有结论时置灰；有了结论就恢复', () => {
    expect(manualCheckBlockedReason(null, eolDone, true, clean)).toContain('svn/base')
    expect(manualCheckBlockedReason(inSync, eolDone, true, clean)).toBeNull()
  })

  it('行尾迁移没做时置灰', () => {
    expect(manualCheckBlockedReason(inSync, eolNeeds, false, clean)).toContain('行尾')
  })

  it('SVN 侧有新内容时置灰：那时体检必然把新内容报成不符，跟进本身就会体检', () => {
    expect(manualCheckBlockedReason(advanced, eolDone, false, clean)).toContain('跟进')
  })
})
