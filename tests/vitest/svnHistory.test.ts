import { describe, expect, it } from 'vitest'
import {
  applyOverlay,
  groupComplete,
  groupSpecs,
  hiddenRevs,
  mergePage,
  overlayTargets,
  type OverlayResult,
  type SvnRevEntry,
} from '@/engine/svnHistory'
import { UNCOMMITTED, type BlameData, type FileHistoryEntry } from '@/engine/fileHistory'

const oid = (c: string) => c.repeat(40)

const entry = (over: Partial<FileHistoryEntry>): FileHistoryEntry => ({
  id: oid('a'),
  parents: [],
  author: 'me',
  time: 100,
  subject: 's',
  path: 'Code/A.cs',
  oldPath: null,
  status: 'modified',
  svn: null,
  ...over,
})

const base = (fileRev: number, deliveredRev: number | null = null) => ({
  baseline: { fileRev, floor: fileRev, deliveredRev },
  delivered: null,
})

const rev = (n: number): SvnRevEntry => ({
  rev: n,
  author: `u${n}`,
  date: '2026-11-20T00:00:00.000000Z',
  message: `m${n}`,
})

describe('groupSpecs / hiddenRevs', () => {
  it('groupSpecs keeps only baselines, newest first', () => {
    const entries = [
      entry({ id: oid('1'), svn: { baseline: null, delivered: { kind: 'undelivered' } } }),
      entry({ id: oid('2'), svn: base(30) }),
      entry({ id: oid('3'), svn: null }),
      entry({ id: oid('4'), path: 'Code/Old.cs', svn: base(10) }),
    ]
    expect(groupSpecs(entries)).toEqual([
      { id: oid('2'), path: 'Code/A.cs', fileRev: 30 },
      { id: oid('4'), path: 'Code/Old.cs', fileRev: 10 },
    ])
  })

  it('hiddenRevs unions baseline deliveredRev and local delivered revs', () => {
    const entries = [
      entry({ id: oid('1'), svn: { baseline: null, delivered: { kind: 'delivered', rev: 7 } } }),
      entry({ id: oid('2'), svn: base(9, 7) }),
      entry({ id: oid('3'), svn: base(5, 4) }),
    ]
    expect(hiddenRevs(entries).sort((a, b) => a - b)).toEqual([4, 7])
  })
})

describe('groupComplete', () => {
  const specs = [
    { id: 'b1', path: 'p', fileRev: 30 },
    { id: 'b2', path: 'p', fileRev: 20 },
    { id: 'b3', path: 'p', fileRev: 10 },
  ]
  it('everything is complete when there is no cursor', () => {
    expect([0, 1, 2].map((i) => groupComplete(specs, i, null))).toEqual([true, true, true])
  })
  it('a group is complete once the cursor is at or below the next older baseline', () => {
    const cursor = { pegPath: 'p', pegRev: 30, nextTop: 15 }
    expect([0, 1, 2].map((i) => groupComplete(specs, i, cursor))).toEqual([true, false, false])
  })
})

describe('mergePage', () => {
  it('mergePage appends and dedupes by rev', () => {
    const first = mergePage({}, { groups: { g1: [rev(30), rev(25)] }, cursor: null, error: null })
    const next = mergePage(first, {
      groups: { g1: [rev(25), rev(21)], g2: [rev(10)] },
      cursor: null,
      error: null,
    })
    expect(next.g1.map((e) => e.rev)).toEqual([30, 25, 21])
    expect(next.g2.map((e) => e.rev)).toEqual([10])
    // 不改原对象：React 状态要换引用
    expect(first.g1.map((e) => e.rev)).toEqual([30, 25])
  })
})

const blameOf = (lines: Array<[string, string, number]>, baselines: Record<string, number>): BlameData => ({
  lines: lines.map(([commit, origPath, origLine], i) => ({
    commit,
    origPath,
    origLine,
    finalLine: i + 1,
    content: `l${i + 1}`,
  })),
  commits: Object.fromEntries(
    [...new Set(lines.map(([c]) => c))].map((c) => [
      c,
      {
        author: 'me',
        time: 1,
        summary: 's',
        previous: null,
        baseline: baselines[c] != null ? { fileRev: baselines[c], floor: baselines[c], deliveredRev: null } : null,
        delivered: null,
      },
    ]),
  ),
  binary: false,
  truncated: false,
  totalLines: lines.length,
})

describe('overlayTargets', () => {
  it('overlayTargets picks the 8 largest baseline groups', () => {
    const lines: Array<[string, string, number]> = []
    // 10 条基线，第 k 条有 k 行
    const baselines: Record<string, number> = {}
    // id 用字母：oid('0') 就是全 0 的 UNCOMMITTED
    for (let k = 1; k <= 10; k++) {
      const id = oid('abcdefghij'[k - 1])
      baselines[id] = k
      for (let i = 0; i < k; i++) lines.push([id, 'Code/A.cs', i + 1])
    }
    const t = overlayTargets(blameOf(lines, baselines))
    expect(t).toHaveLength(8)
    expect(t.map((x) => x.fileRev)).toEqual([10, 9, 8, 7, 6, 5, 4, 3])
  })

  it('overlayTargets ignores non-baseline and uncommitted lines', () => {
    const b = blameOf(
      [
        [oid('b'), 'Code/A.cs', 1],
        [oid('c'), 'Code/A.cs', 2],
        [UNCOMMITTED, 'Code/A.cs', 3],
      ],
      { [oid('b')]: 5 },
    )
    expect(overlayTargets(b)).toEqual([{ baselineId: oid('b'), origPath: 'Code/A.cs', fileRev: 5 }])
  })
})

describe('applyOverlay', () => {
  const b = blameOf(
    [
      [oid('b'), 'Code/A.cs', 1],
      [oid('b'), 'Code/A.cs', 2],
      [oid('c'), 'Code/A.cs', 1],
      [oid('d'), 'Code/A.cs', 4],
    ],
    { [oid('b')]: 5, [oid('d')]: 3 },
  )
  const targets = [
    { baselineId: oid('b'), origPath: 'Code/A.cs', fileRev: 5 },
    { baselineId: oid('d'), origPath: 'Code/A.cs', fileRev: 3 },
  ]

  it('applyOverlay maps baseline lines by origLine and leaves git lines alone', () => {
    const results: OverlayResult[] = [
      {
        kind: 'mapped',
        lines: [
          { rev: 2, author: 'ann', date: 'd2' },
          { rev: 5, author: 'bob', date: 'd5' },
        ],
      },
      { kind: 'mapped', lines: [null, null, null, { rev: 1, author: 'cat', date: 'd1' }] },
    ]
    const v = applyOverlay(b, targets, results)
    expect(v.lines).toEqual([
      { kind: 'svn', rev: 2, author: 'ann', date: 'd2' },
      { kind: 'svn', rev: 5, author: 'bob', date: 'd5' },
      { kind: 'git' },
      { kind: 'svn', rev: 1, author: 'cat', date: 'd1' },
    ])
    expect(v.unmapped).toBe(0)
    expect(v.failure).toBeNull()
  })

  it('applyOverlay marks mismatch groups as baseline and counts them', () => {
    const results: OverlayResult[] = [
      { kind: 'mismatch', svnLines: 1, gitLines: 3 },
      { kind: 'failed', error: { code: 'svn_unreachable', message: 'x' } },
    ]
    const v = applyOverlay(b, targets, results)
    expect(v.lines.map((l) => l.kind)).toEqual(['baseline', 'baseline', 'git', 'baseline'])
    expect(v.unmapped).toBe(2)
    expect(v.failure?.code).toBe('svn_unreachable')
  })

  it('baseline lines that were not targeted count as unmapped', () => {
    const v = applyOverlay(b, targets.slice(0, 1), [
      { kind: 'mapped', lines: [{ rev: 2, author: 'a', date: 'd' }, { rev: 2, author: 'a', date: 'd' }] },
    ])
    expect(v.lines[3]).toEqual({ kind: 'baseline' })
    expect(v.unmapped).toBe(1)
  })
})
