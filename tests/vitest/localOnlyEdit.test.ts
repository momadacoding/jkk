import { describe, expect, it } from 'vitest'
import {
  describeLocalOnlyEditDone,
  describeLocalOnlyToggle,
  splitSelectionProblem,
} from '@/engine/localOnlyEdit'
import { commitMenu } from '@/engine/commitActions'
import { ALL_CAPABILITIES, NO_GIT_ONLY_CAPABILITIES } from '@/engine/capabilities'
import type { CommitNode } from '@/engine/types'

const commit = (over: Partial<CommitNode> = {}): CommitNode =>
  ({
    id: 'a'.repeat(40),
    parents: ['b'.repeat(40)],
    author: 'me',
    time: 1_700_000_000,
    message: 'add @avatarclass',
    ...over,
  }) as CommitNode

const actions = (c: CommitNode, caps = NO_GIT_ONLY_CAPABILITIES) =>
  commitMenu(c, { isHead: false, caps }).map((e) => e.action)

describe('graph 右键菜单里的「仅本地」三项（使用报告 9.2）', () => {
  it('Local-Only commit：取消、拆分', () => {
    const a = actions(commit({ localOnly: true }))
    expect(a).toContain('local-only-off')
    expect(a).toContain('local-only-split')
    expect(a).not.toContain('local-only-on')
  })

  it('普通 commit：标为仅本地，没有拆分', () => {
    const a = actions(commit())
    expect(a).toContain('local-only-on')
    expect(a).not.toContain('local-only-off')
    expect(a).not.toContain('local-only-split')
  })

  it('普通 git 仓库没有这几项：这个标记只在 SVN 混合仓库里有意义', () => {
    const a = actions(commit({ localOnly: true }), ALL_CAPABILITIES)
    expect(a.filter((x) => x.startsWith('local-only'))).toEqual([])
  })

  it('SVN 节点与基线 commit 上没有这几项：它们不在工作分支的本地提交里', () => {
    for (const c of [
      commit({ svnRev: 7 }),
      commit({ baselineRev: 7 }),
      commit({ baselineFloor: 7 }),
    ]) {
      expect(actions(c).filter((x) => x.startsWith('local-only'))).toEqual([])
    }
  })
})

describe('describeLocalOnlyToggle', () => {
  it('确认框说清楚：改写的是这个及之后的 commit、工作区不动、别的分支不动', () => {
    const m = describeLocalOnlyToggle(commit({ localOnly: true }), false)
    expect(m.title).toBe('取消「仅本地」')
    expect(m.message).toContain('add @avatarclass')
    expect(m.warning).toContain('之后的 commit')
    expect(m.warning).toContain('工作区')
    expect(m.warning).toContain('别的分支')
    expect(describeLocalOnlyToggle(commit(), true).title).toBe('标为「仅本地」')
  })

  it('取消时写明后果：之后交付会把它一起提交上 SVN', () => {
    expect(describeLocalOnlyToggle(commit({ localOnly: true }), false).message).toContain('交付')
  })
})

describe('describeLocalOnlyEditDone', () => {
  const done = { branch: 'work', oldTip: 'o'.repeat(40), tip: 't'.repeat(40), commit: 'c'.repeat(40), otherBranches: [] as string[] }

  it('没有别的分支含着它：不打扰', () => {
    expect(describeLocalOnlyEditDone(done)).toBeNull()
  })

  it('别的分支上也有这个 commit：说明它们留着旧的', () => {
    const n = describeLocalOnlyEditDone({ ...done, otherBranches: ['feature', 'old'] })!
    expect(n.message).toContain('feature')
    expect(n.message).toContain('old')
    expect(n.message).toContain('旧的')
  })
})

describe('splitSelectionProblem', () => {
  it('选了一部分：可以拆', () => {
    expect(splitSelectionProblem(3, 1)).toBeNull()
    expect(splitSelectionProblem(3, 2)).toBeNull()
  })

  it('一个都没选、全选了：不能拆，并说明为什么', () => {
    expect(splitSelectionProblem(3, 0)).toContain('选')
    expect(splitSelectionProblem(3, 3)).toContain('取消')
  })

  it('只改了一个文件的 commit 没法拆', () => {
    expect(splitSelectionProblem(1, 0)).toContain('只改了一个文件')
    expect(splitSelectionProblem(1, 1)).toContain('只改了一个文件')
  })
})
