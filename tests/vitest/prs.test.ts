import { describe, expect, it } from 'vitest'
import { indexPullRequestsByBranch } from '../../src/engine/prs'
import type { PullRequest } from '../../src/engine/types'

const pr = (number: number, headRefName: string, extra: Partial<PullRequest> = {}): PullRequest => ({
  number,
  headRefName,
  title: `PR ${number}`,
  isDraft: false,
  url: `https://github.com/o/r/pull/${number}`,
  ...extra,
})

describe('indexPullRequestsByBranch', () => {
  it('空数组给出空索引，不抛异常', () => {
    expect(indexPullRequestsByBranch([]).size).toBe(0)
  })

  it('按 headRefName 建索引', () => {
    const index = indexPullRequestsByBranch([pr(36, 'fix/watch'), pr(35, 'feat/tab')])
    expect(index.get('fix/watch')?.number).toBe(36)
    expect(index.get('feat/tab')?.number).toBe(35)
  })

  it('分支名里的斜杠原样保留，不做任何切分', () => {
    const index = indexPullRequestsByBranch([pr(7, 'feat/a/b/c')])
    expect(index.get('feat/a/b/c')?.number).toBe(7)
    expect(index.get('feat')).toBeUndefined()
  })

  it('同一分支有多个 open PR 时取编号最大的那个', () => {
    // GitHub 允许同一个 head 分支同时开多个 PR（指向不同 base）。
    // 徽章只有一个位置，取最新的那条最贴近「我刚开的 PR」。
    const index = indexPullRequestsByBranch([pr(10, 'feat/x'), pr(42, 'feat/x'), pr(31, 'feat/x')])
    expect(index.get('feat/x')?.number).toBe(42)
  })

  it('顺序颠倒也取编号最大的，不依赖 gh 的返回顺序', () => {
    const index = indexPullRequestsByBranch([pr(42, 'feat/x'), pr(10, 'feat/x')])
    expect(index.get('feat/x')?.number).toBe(42)
  })

  it('大小写不同视为不同分支，不做大小写折叠', () => {
    // GitHub 的分支名是大小写敏感的，折叠会把 Feat/X 的 PR 错挂到 feat/x 上。
    // 宁可少显示一个徽章，也不能指着错的 PR。
    const index = indexPullRequestsByBranch([pr(5, 'Feat/X')])
    expect(index.get('feat/x')).toBeUndefined()
    expect(index.get('Feat/X')?.number).toBe(5)
  })

  it('headRefName 为空或只有空白的条目直接丢掉', () => {
    // 空键会和「分支名为空」的查询意外命中，而后者本身就是坏数据
    const index = indexPullRequestsByBranch([pr(1, ''), pr(2, '   '), pr(3, 'ok')])
    expect(index.size).toBe(1)
    expect(index.get('ok')?.number).toBe(3)
  })

  it('保留 draft 标记和 url，徽章要用', () => {
    const index = indexPullRequestsByBranch([pr(9, 'feat/d', { isDraft: true })])
    expect(index.get('feat/d')?.isDraft).toBe(true)
    expect(index.get('feat/d')?.url).toBe('https://github.com/o/r/pull/9')
  })
})
