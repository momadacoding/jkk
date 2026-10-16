import { describe, expect, it } from 'vitest'
import { computeLaneBranches, formatLaneBranch } from '../../src/engine/laneBranch'
import { WORKDIR_ID } from '../../src/engine/owners'
import type { CommitNode, RefPointer } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

function ref(name: string, hash: string, kind: RefPointer['kind']): RefPointer {
  return { name, hash, kind, ahead: null, behind: null }
}

describe('computeLaneBranches', () => {
  it('主干车道上的老提交只报 master，不报从它切出去的分支', () => {
    const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/feature', 'f1', 'branch'),
      ref('refs/heads/master', 'm1', 'branch'),
    ]
    const lanes = computeLaneBranches(commits, refs)
    expect(lanes.get('m0')).toEqual({ kind: 'branch', names: ['master'] })
    expect(lanes.get('f1')).toEqual({ kind: 'branch', names: ['feature'] })
  })

  it('分支 tip 以下、分叉点以上的提交都归这条分支', () => {
    const commits = [node('m1', ['m0']), node('f2', ['f1']), node('f1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature/x', 'f2', 'branch'),
    ]
    const lanes = computeLaneBranches(commits, refs)
    expect(lanes.get('f1')).toEqual({ kind: 'branch', names: ['feature/x'] })
  })

  it('本地分支与同一提交上的同名远程分支只报本地名', () => {
    const commits = [node('m1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/remotes/origin/master', 'm1', 'remote'),
    ]
    expect(computeLaneBranches(commits, refs).get('m0')).toEqual({ kind: 'branch', names: ['master'] })
  })

  it('只有远程分支的车道报带 remote 前缀的名字', () => {
    const commits = [node('m1', ['m0']), node('r1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/remotes/origin/feat', 'r1', 'remote'),
    ]
    expect(computeLaneBranches(commits, refs).get('r1')).toEqual({ kind: 'branch', names: ['origin/feat'] })
  })

  it('同一个 tip 上挂着几个不同名字的分支时全部列出，tag 不算分支', () => {
    const commits = [node('m1', ['m0']), node('a1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/a', 'a1', 'branch'),
      ref('refs/tags/v1', 'a1', 'tag'),
      ref('refs/heads/b', 'a1', 'branch'),
    ]
    expect(computeLaneBranches(commits, refs).get('a1')).toEqual({ kind: 'branch', names: ['a', 'b'] })
  })

  it('合并后被删掉的分支报「已合并进」合并它的那条分支', () => {
    // feature 合进 master 后删了：f1、f2 仍独占一条车道，但顶上没有名字
    const commits = [
      node('M', ['m1', 'f2']),
      node('f2', ['f1']),
      node('f1', ['m0']),
      node('m1', ['m0']),
      node('m0', []),
    ]
    const refs = [ref('refs/heads/master', 'M', 'branch')]
    const lanes = computeLaneBranches(commits, refs)
    expect(lanes.get('f1')).toEqual({ kind: 'merged', into: ['master'] })
    expect(lanes.get('f2')).toEqual({ kind: 'merged', into: ['master'] })
  })

  it('合进去的那条分支也删了时，往上找到第一个还有名字的分支', () => {
    // B 合进 A，A 合进 master，A、B 都删了
    const commits = [
      node('M', ['m0', 'A2']),
      node('A2', ['A1', 'B1']),
      node('B1', ['m0']),
      node('A1', ['m0']),
      node('m0', []),
    ]
    const refs = [ref('refs/heads/master', 'M', 'branch')]
    const lanes = computeLaneBranches(commits, refs)
    expect(lanes.get('A1')).toEqual({ kind: 'merged', into: ['master'] })
    expect(lanes.get('B1')).toEqual({ kind: 'merged', into: ['master'] })
  })

  it('同一条已删分支先后合进两条分支时两个都报，离它最近的那次合并排前面', () => {
    const commits = [
      node('M2', ['M1', 'f1']),
      node('D', ['m0', 'f1']),
      node('M1', ['m0']),
      node('f1', ['m0']),
      node('m0', []),
    ]
    const refs = [
      ref('refs/heads/master', 'M2', 'branch'),
      ref('refs/heads/dev', 'D', 'branch'),
    ]
    // D 在 M2 之后一行，更老，是更早的那次合并
    expect(computeLaneBranches(commits, refs).get('f1')).toEqual({ kind: 'merged', into: ['dev', 'master'] })
  })

  it('游离 HEAD 上的提交没有分支', () => {
    const commits = [node('d1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/master', 'm0', 'branch'),
      ref('HEAD', 'd1', 'head'),
    ]
    expect(computeLaneBranches(commits, refs).get('d1')).toEqual({ kind: 'none' })
  })

  it('只挂着 tag 的车道没有分支', () => {
    const commits = [node('m1', ['m0']), node('t1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/tags/v1', 't1', 'tag'),
    ]
    expect(computeLaneBranches(commits, refs).get('t1')).toEqual({ kind: 'none' })
  })

  it('SVN 主线上的节点报 SVN 主线', () => {
    const commits = [node('svn:2', ['svn:1']), node('svn:1', [])]
    expect(computeLaneBranches(commits, []).get('svn:1')).toEqual({ kind: 'svn' })
  })

  it('工作区伪行与图外提交查不到', () => {
    const commits = [node(WORKDIR_ID, ['m0']), node('m0', [])]
    const refs = [ref('refs/heads/master', 'm0', 'branch')]
    const lanes = computeLaneBranches(commits, refs)
    expect(lanes.get(WORKDIR_ID)).toBeUndefined()
    expect(lanes.get('not-in-graph')).toBeUndefined()
    expect(lanes.get('m0')).toEqual({ kind: 'branch', names: ['master'] })
  })

  it('空图返回空表', () => {
    expect(computeLaneBranches([], []).size).toBe(0)
  })
})

describe('formatLaneBranch', () => {
  it('各种归属的显示文字', () => {
    expect(formatLaneBranch({ kind: 'branch', names: ['master'] })).toBe('master')
    expect(formatLaneBranch({ kind: 'branch', names: ['a', 'b'] })).toBe('a, b')
    expect(formatLaneBranch({ kind: 'merged', into: ['master'] })).toBe('已合并进 master')
    expect(formatLaneBranch({ kind: 'merged', into: ['dev', 'master'] })).toBe('已合并进 dev, master')
    expect(formatLaneBranch({ kind: 'svn' })).toBe('SVN 主线')
    expect(formatLaneBranch({ kind: 'none' })).toBe('无分支')
  })
})
