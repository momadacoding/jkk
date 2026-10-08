import { describe, expect, it } from 'vitest'
import { mergedPullRequestsFor, unmergedNote } from '../../src/engine/deleteBranchNote'
import type { PullRequest } from '../../src/engine/types'

const PR_HEAD = '93e3d222de3626f1f95b6e5de682b6cc22c48278'

const pr = (number: number, headRefName: string, extra: Partial<PullRequest> = {}): PullRequest => ({
  number,
  headRefName,
  title: `PR ${number}`,
  isDraft: false,
  state: 'MERGED',
  headRefOid: PR_HEAD,
  url: `https://github.com/o/r/pull/${number}`,
  ...extra,
})

describe('mergedPullRequestsFor', () => {
  it('只取同名分支上已合并、且带合并点的 PR', () => {
    const prs = [
      pr(321, 'worktree-ios-kickoff'),
      pr(300, 'worktree-ios-kickoff', { state: 'OPEN' }),
      pr(299, 'worktree-ios-kickoff', { state: 'CLOSED' }),
      pr(298, 'worktree-ios-kickoff', { headRefOid: '' }),
      pr(297, 'other'),
    ]
    expect(mergedPullRequestsFor(prs, 'worktree-ios-kickoff').map((p) => p.number)).toEqual([321])
  })

  it('分支名精确匹配，不做大小写折叠', () => {
    // 与 indexPullRequestsByBranch 同一约定：GitHub 分支名大小写敏感
    expect(mergedPullRequestsFor([pr(1, 'Feat/X')], 'feat/x')).toEqual([])
  })

  it('编号大的排前面：后端只试前若干个，最新的合并最可能是那一个', () => {
    const prs = [pr(5, 'a', { headRefOid: '1'.repeat(40) }), pr(9, 'a'), pr(7, 'a', { headRefOid: '2'.repeat(40) })]
    expect(mergedPullRequestsFor(prs, 'a').map((p) => p.number)).toEqual([9, 7, 5])
  })

  it('空列表、空分支名安全返回空数组', () => {
    expect(mergedPullRequestsFor([], 'a')).toEqual([])
    expect(mergedPullRequestsFor([pr(1, '')], '')).toEqual([])
  })
})

describe('unmergedNote', () => {
  const base = { unmerged: 13, comparedTo: 'main', contentMerged: false, mergedVia: null }

  it('没有未合并提交时什么都不显示', () => {
    expect(unmergedNote({ ...base, unmerged: 0 }, [])).toBeNull()
  })

  it('真有未合并提交：红色警告，说只能靠 reflog 找回', () => {
    const note = unmergedNote(base, [])
    expect(note?.tone).toBe('warning')
    expect(note?.text).toContain('13 个提交不在 main 上')
    expect(note?.text).toContain('reflog')
  })

  it('分支自己就认出了 squash：中性提示', () => {
    const note = unmergedNote({ ...base, contentMerged: true }, [])
    expect(note?.tone).toBe('note')
    expect(note?.text).toContain('删掉不会丢东西')
    expect(note?.text).not.toContain('reflog')
  })

  it('靠 PR 合并点认出来的：说清楚是哪个 PR、合并点比本地新', () => {
    // 真实案例：本地 worktree-ios-kickoff 落后 PR 321 的合并点 93e3d22 一个提交，
    // 旧弹框喊「13 个提交不在 main 上，删除后只能靠 reflog 找回」
    const note = unmergedNote(
      { ...base, contentMerged: true, mergedVia: PR_HEAD },
      [pr(321, 'worktree-ios-kickoff')],
    )
    expect(note?.tone).toBe('note')
    expect(note?.text).toContain('#321')
    expect(note?.text).toContain('93e3d22')
    expect(note?.text).toContain('删掉不会丢东西')
  })

  it('说的是内容实际进了哪里，不是报数用的 comparedTo', () => {
    // 叠 PR：合进的是 feat/a，不是 main。照 comparedTo 说「进了 main」就是在说谎
    const note = unmergedNote(
      { ...base, contentMerged: true, mergedVia: PR_HEAD, mergedInto: 'feat/a' },
      [pr(321, 'a')],
    )
    expect(note?.text).toContain('改动已经进了 feat/a')
    expect(note?.text).not.toContain('进了 main')
  })

  it('mergedVia 比对不分大小写', () => {
    const note = unmergedNote(
      { ...base, contentMerged: true, mergedVia: PR_HEAD.toUpperCase() },
      [pr(321, 'a')],
    )
    expect(note?.text).toContain('#321')
  })

  it('找不到对应 PR 时仍给中性提示，只报合并点', () => {
    // 后端已经在本地证明过内容在主干上，PR 列表对不上不影响结论
    const note = unmergedNote({ ...base, contentMerged: true, mergedVia: PR_HEAD }, [])
    expect(note?.tone).toBe('note')
    expect(note?.text).toContain('93e3d22')
    expect(note?.text).not.toContain('#')
  })

  it('老后端没有 mergedVia 字段时按分支自己认出处理', () => {
    const legacy: Parameters<typeof unmergedNote>[0] = { unmerged: 2, comparedTo: 'main', contentMerged: true }
    expect(unmergedNote(legacy, [])?.tone).toBe('note')
  })
})
