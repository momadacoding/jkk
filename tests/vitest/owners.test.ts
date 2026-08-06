import { describe, expect, it } from 'vitest'
import { collectTips, computeOwners } from '../../src/engine/owners'
import type { CommitNode, RefPointer } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

function ref(name: string, hash: string, kind: RefPointer['kind']): RefPointer {
  return { name, hash, kind }
}

describe('collectTips', () => {
  it('把 master 排在最前面，哪怕它的 tip 比别的分支旧', () => {
    // feature 骑在 master 头上：feature 行号更小（更新），但 master 必须先被认领
    const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/feature', 'f1', 'branch'),
      ref('refs/heads/master', 'm1', 'branch'),
    ]
    expect(collectTips(commits, refs)[0]).toBe('m1')
  })

  it('没有本地 master 时回退到 origin/master', () => {
    const commits = [node('f1', ['m1']), node('m1', [])]
    const refs = [
      ref('refs/heads/feature', 'f1', 'branch'),
      ref('refs/remotes/origin/master', 'm1', 'remote'),
    ]
    expect(collectTips(commits, refs)[0]).toBe('m1')
  })

  it('本地和远程 master 都没有时，lane 0 归下一级的第一个 tip', () => {
    const commits = [node('d1', ['d0']), node('d0', [])]
    const refs = [ref('refs/heads/develop', 'd1', 'branch')]
    expect(collectTips(commits, refs)[0]).toBe('d1')
  })

  it('忽略 kind=head 的 ref —— 否则 checkout 会改变整张图', () => {
    const commits = [node('f1', ['m1']), node('m1', [])]
    // 后端 refs.rs 总会额外塞这条；它指向 f1，若参与排序就会抢在 master 前面
    const withHead = [
      ref('HEAD', 'f1', 'head'),
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const withoutHead = withHead.filter((r) => r.kind !== 'head')
    expect(collectTips(commits, withHead)).toEqual(collectTips(commits, withoutHead))
    expect(collectTips(commits, withHead)[0]).toBe('m1')
  })

  it('同一个 hash 上的本地与远程分支只算一个 tip', () => {
    const commits = [node('m1', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/remotes/origin/master', 'm1', 'remote'),
    ]
    expect(collectTips(commits, refs)).toEqual(['m1'])
  })

  it('把没有子节点的悬空提交也算作 tip', () => {
    // dangling 没有任何 ref，也没有子节点，不认领它就会没有主人
    const commits = [node('dangling', ['m0']), node('m1', ['m0']), node('m0', [])]
    const refs = [ref('refs/heads/master', 'm1', 'branch')]
    expect(collectTips(commits, refs)).toContain('dangling')
  })

  it('跳过指向图外提交的 ref', () => {
    // 5000 条上限截断后，老分支的 tip 可能不在这批数据里
    const commits = [node('m1', [])]
    const refs = [ref('refs/heads/old', 'not-in-graph', 'branch')]
    expect(collectTips(commits, refs)).toEqual(['m1'])
  })
})

describe('computeOwners', () => {
  it('feature 与 master 属于不同 owner，分叉点归 master', () => {
    const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const { ownerOf } = computeOwners(commits, refs)
    expect(ownerOf.get('m1')).toBe(0)
    expect(ownerOf.get('m0')).toBe(0)
    expect(ownerOf.get('f1')).not.toBe(0)
  })

  it('owner 0 的行区间覆盖它整条第一父链', () => {
    const commits = [node('m2', ['m1']), node('m1', ['m0']), node('m0', [])]
    const refs = [ref('refs/heads/master', 'm2', 'branch')]
    const { owners } = computeOwners(commits, refs)
    expect(owners[0]).toEqual({ index: 0, minRow: 0, maxRow: 2 })
  })

  it('合并提交的第二父会就地开一个新 owner', () => {
    // merge 的第二父 b1 只能通过第二父边到达，第一父链遍历覆盖不到它
    const commits = [node('merge', ['m1', 'b1']), node('b1', ['m0']), node('m1', ['m0']), node('m0', [])]
    const refs = [ref('refs/heads/master', 'merge', 'branch')]
    const { ownerOf } = computeOwners(commits, refs)
    expect(ownerOf.get('b1')).toBeDefined()
    expect(ownerOf.get('b1')).not.toBe(ownerOf.get('m1'))
  })

  it('每一个提交都有主人', () => {
    const commits = [
      node('merge', ['m1', 'b2']),
      node('b2', ['b1']),
      node('m1', ['m0']),
      node('b1', ['m0']),
      node('m0', []),
    ]
    const refs = [ref('refs/heads/master', 'merge', 'branch')]
    const { ownerOf } = computeOwners(commits, refs)
    for (const c of commits) expect(ownerOf.has(c.id)).toBe(true)
  })

  it('父提交不在数据里时停止遍历而不是崩溃', () => {
    const commits = [node('c1', ['truncated'])]
    const { ownerOf } = computeOwners(commits, [])
    expect(ownerOf.get('c1')).toBe(0)
    expect(ownerOf.has('truncated')).toBe(false)
  })
})
