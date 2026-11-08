import { describe, expect, it } from 'vitest'
import {
  UNCOMMITTED,
  blameBlocks,
  blameHeat,
  blameRevForEntry,
  describeBlameRev,
  historyTargetForCommit,
  historyTargetForWorkdir,
  initialEntryId,
  isFullOid,
  previousBlameRev,
  workdirHistoryAvailable,
  graphSelectionFor,
  type BlameCommit,
  type BlameLine,
  type FileHistoryEntry,
} from '@/engine/fileHistory'
import type { CommitNode } from '@/engine/types'

const OID_A = 'a'.repeat(40)
const OID_B = 'b'.repeat(40)

const entry = (over: Partial<FileHistoryEntry>): FileHistoryEntry => ({
  id: OID_A,
  parents: [OID_B],
  author: 'me',
  time: 100,
  subject: 's',
  path: 'f.txt',
  oldPath: null,
  status: 'modified',
  ...over,
})

describe('isFullOid', () => {
  it('只认 40 位十六进制', () => {
    expect(isFullOid(OID_A)).toBe(true)
    expect(isFullOid('ABCDEF'.repeat(6) + 'abcd')).toBe(true)
    expect(isFullOid('svn:12')).toBe(false)
    expect(isFullOid(OID_A.slice(1))).toBe(false)
    expect(isFullOid('')).toBe(false)
  })
})

describe('historyTargetForCommit', () => {
  it('起点是 HEAD 加这个提交（决策 D2），选中它，Blame 看这一版', () => {
    const t = historyTargetForCommit('src/a.ts', { id: OID_A }, 'modified', 'history')
    expect(t.revs).toEqual(['HEAD', OID_A])
    expect(t.focus).toBe(OID_A)
    expect(t.blame).toEqual({ rev: OID_A, path: 'src/a.ts' })
    expect(t.tab).toBe('history')
  })

  it('这个提交删了文件：Blame 看它的父提交里那一版', () => {
    const t = historyTargetForCommit('a.ts', { id: OID_A }, 'deleted', 'blame')
    expect(t.blame).toEqual({ rev: `${OID_A}^`, path: 'a.ts' })
    expect(t.tab).toBe('blame')
  })

  it('svn 节点没有本地基线：只从 HEAD 走，不选中、Blame 看 HEAD', () => {
    const t = historyTargetForCommit('a.ts', { id: 'svn:16' }, 'modified', 'history')
    expect(t.revs).toEqual(['HEAD'])
    expect(t.focus).toBeNull()
    expect(t.blame).toEqual({ rev: 'HEAD', path: 'a.ts' })
  })

  it('svn 节点有本地基线：按基线提交算；删除时 Blame 前一版基线', () => {
    const svn = { id: 'svn:16', baselineOid: OID_A, diffBase: OID_B }
    const t = historyTargetForCommit('a.ts', svn, 'modified', 'history')
    expect(t.revs).toEqual(['HEAD', OID_A])
    expect(t.focus).toBe(OID_A)
    expect(t.blame).toEqual({ rev: OID_A, path: 'a.ts' })
    expect(historyTargetForCommit('a.ts', svn, 'deleted', 'blame').blame).toEqual({ rev: OID_B, path: 'a.ts' })
    // 只有这一版基线、没有前一版：退回基线的父提交
    expect(historyTargetForCommit('a.ts', { id: 'svn:16', baselineOid: OID_A }, 'deleted', 'blame').blame)
      .toEqual({ rev: `${OID_A}^`, path: 'a.ts' })
  })
})

describe('historyTargetForWorkdir', () => {
  it('从 Local Changes 进来：只从 HEAD 走，Blame 看工作区', () => {
    const t = historyTargetForWorkdir('a.ts', 'modified', 'blame')
    expect(t.revs).toEqual(['HEAD'])
    expect(t.focus).toBeNull()
    expect(t.blame).toEqual({ rev: null, path: 'a.ts' })
  })
  it('工作区里删掉了：Blame 看 HEAD 里那一版', () => {
    expect(historyTargetForWorkdir('a.ts', 'deleted', 'blame').blame).toEqual({ rev: 'HEAD', path: 'a.ts' })
  })
})

describe('workdirHistoryAvailable', () => {
  it('新增的文件 HEAD 里还没有，其余都可以', () => {
    expect(workdirHistoryAvailable('added')).toBe(false)
    for (const s of ['modified', 'deleted', 'conflicted', 'renamed'] as const) {
      expect(workdirHistoryAvailable(s)).toBe(true)
    }
    expect(workdirHistoryAvailable(null)).toBe(true)
  })
})

describe('initialEntryId', () => {
  const list = [entry({ id: OID_A }), entry({ id: OID_B })]
  it('focus 在列表里就选它，否则选第一条，空列表为 null', () => {
    expect(initialEntryId(list, OID_B)).toBe(OID_B)
    expect(initialEntryId(list, 'c'.repeat(40))).toBe(OID_A)
    expect(initialEntryId(list, null)).toBe(OID_A)
    expect(initialEntryId([], OID_A)).toBeNull()
  })
})

describe('blameRevForEntry', () => {
  it('普通一条：那个提交、那时的路径', () => {
    expect(blameRevForEntry(entry({ path: 'old.txt' }))).toEqual({ rev: OID_A, path: 'old.txt' })
  })
  it('删除那一条：父提交里那一版；根提交删除（不可能有）时为 null', () => {
    expect(blameRevForEntry(entry({ status: 'deleted' }))).toEqual({ rev: OID_B, path: 'f.txt' })
    expect(blameRevForEntry(entry({ status: 'deleted', parents: [] }))).toBeNull()
  })
})

describe('previousBlameRev', () => {
  const info = (previous: BlameCommit['previous']): BlameCommit => ({ author: 'a', time: 1, summary: 's', previous })
  it('有 previous 就 Blame 那一版的那个路径，没有为 null', () => {
    expect(previousBlameRev(info({ commit: OID_B, path: 'o.txt' }))).toEqual({ rev: OID_B, path: 'o.txt' })
    expect(previousBlameRev(info(null))).toBeNull()
    expect(previousBlameRev(undefined)).toBeNull()
  })
})

const line = (commit: string, n: number): BlameLine => ({
  commit, origLine: n, finalLine: n, origPath: 'f.txt', content: `l${n}`,
})

describe('blameBlocks', () => {
  it('同一提交连续的行合成一块，隔开的同一提交是两块', () => {
    const blocks = blameBlocks([line('A', 1), line('A', 2), line('B', 3), line('A', 4)])
    expect(blocks).toEqual([
      { commit: 'A', start: 0, length: 2 },
      { commit: 'B', start: 2, length: 1 },
      { commit: 'A', start: 3, length: 1 },
    ])
  })
  it('空输入', () => {
    expect(blameBlocks([])).toEqual([])
  })
})

describe('blameHeat', () => {
  const c = (time: number): BlameCommit => ({ author: 'a', time, summary: '', previous: null })
  it('按时间排名：最新 1、最旧 0，未提交算最新', () => {
    const heat = blameHeat({ old: c(10), mid: c(20), new: c(30), [UNCOMMITTED]: c(5) })
    expect(heat.get('old')).toBe(0)
    expect(heat.get('new')).toBeCloseTo(2 / 3)
    expect(heat.get(UNCOMMITTED)).toBe(1)
    expect(heat.get('mid')).toBeCloseTo(1 / 3)
  })
  it('只有一个提交时是 1，不除以 0', () => {
    expect(blameHeat({ only: c(1) }).get('only')).toBe(1)
  })
  it('同一时间的提交热度相同', () => {
    const heat = blameHeat({ a: c(10), b: c(10), z: c(20) })
    expect(heat.get('a')).toBe(heat.get('b'))
  })
})

describe('graphSelectionFor', () => {
  const node = (over: Partial<CommitNode>): CommitNode => ({ id: OID_A, parents: [], author: 'a', time: 1, message: 'm', ...over })

  it('普通仓库：在图上就是它自己，不在为 null', () => {
    const commits = [node({ id: OID_A })]
    expect(graphSelectionFor(OID_A, commits, null)).toBe(OID_A)
    expect(graphSelectionFor(OID_B, commits, null)).toBeNull()
  })

  it('混合仓库：基线提交换成接合后的 svn:N 节点', () => {
    const gitCommits = [node({ id: OID_B, baselineRev: 16 })]
    const commits = [node({ id: 'svn:16', vcs: 'svn', svnRev: 16 })]
    expect(graphSelectionFor(OID_B, commits, gitCommits)).toBe('svn:16')
  })

  it('混合仓库：交付过、收在 svn 节点里的本地提交也能选中（独立 review 第 1 条）', () => {
    const delivered = node({ id: OID_A, message: '本地提交' })
    const commits = [node({ id: 'svn:16', vcs: 'svn', svnRev: 16, deliveredFrom: [delivered] })]
    expect(graphSelectionFor(OID_A, commits, [])).toBe(OID_A)
  })
})

describe('describeBlameRev', () => {
  const list = [entry({ id: OID_A, subject: '改了标题' })]
  it('工作区 / 历史里有的写标题 / 父提交写明「之前」/ 其余写短 hash', () => {
    expect(describeBlameRev({ rev: null, path: 'f' }, list)).toBe('工作区（含未提交的改动）')
    expect(describeBlameRev({ rev: OID_A, path: 'f' }, list)).toBe('aaaaaaa 改了标题')
    expect(describeBlameRev({ rev: `${OID_A}^`, path: 'f' }, list)).toBe('aaaaaaa 之前的版本')
    expect(describeBlameRev({ rev: OID_B, path: 'f' }, list)).toBe('bbbbbbb')
    expect(describeBlameRev({ rev: 'HEAD', path: 'f' }, list)).toBe('HEAD')
  })
})
