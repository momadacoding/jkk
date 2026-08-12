import { describe, it, expect } from 'vitest'
import { sortTags, tagShortName } from '@/engine/tagOrder'
import type { CommitNode, RefPointer } from '@/engine/types'

function node(id: string, time: number): CommitNode {
  return { id, parents: [], author: 'a', time, message: id }
}

function tag(name: string, hash: string): RefPointer {
  return { name: `refs/tags/${name}`, hash, kind: 'tag', ahead: null, behind: null }
}

const names = (refs: RefPointer[]) => refs.map((r) => tagShortName(r.name))

describe('tagShortName', () => {
  it('去掉 refs/tags/ 前缀', () => {
    expect(tagShortName('refs/tags/v0.1.0')).toBe('v0.1.0')
  })

  it('名字里含 tags 字样时不会被误伤', () => {
    expect(tagShortName('refs/tags/release/tags-v1')).toBe('release/tags-v1')
  })
})

describe('sortTags', () => {
  it('空数组不炸', () => {
    expect(sortTags([], [])).toEqual([])
  })

  it('按目标提交时间倒序，新的在上', () => {
    const commits = [node('c3', 300), node('c2', 200), node('c1', 100)]
    const tags = [tag('old', 'c1'), tag('new', 'c3'), tag('mid', 'c2')]
    expect(names(sortTags(tags, commits))).toEqual(['new', 'mid', 'old'])
  })

  it('两位数版本号不再被字典序排错', () => {
    // 字典序下 v0.10.0 会排在 v0.2.0 前面，而它其实更新
    const commits = [node('c2', 200), node('c1', 100)]
    const tags = [tag('v0.10.0', 'c2'), tag('v0.2.0', 'c1')]
    expect(names(sortTags(tags, commits))).toEqual(['v0.10.0', 'v0.2.0'])
  })

  it('目标提交不在图里的排到最后', () => {
    const commits = [node('c1', 100)]
    const tags = [tag('dangling', 'nope'), tag('real', 'c1')]
    expect(names(sortTags(tags, commits))).toEqual(['real', 'dangling'])
  })

  it('全都不在图里时仍按名字稳定排序', () => {
    const tags = [tag('a', 'x'), tag('b', 'y')]
    expect(names(sortTags(tags, []))).toEqual(['b', 'a'])
  })

  it('同一条提交上的多个 tag 按名字倒序，顺序稳定', () => {
    const commits = [node('c1', 100)]
    const tags = [tag('alpha', 'c1'), tag('beta', 'c1')]
    expect(names(sortTags(tags, commits))).toEqual(['beta', 'alpha'])
  })

  it('不改动传入的数组', () => {
    const commits = [node('c2', 200), node('c1', 100)]
    const tags = [tag('old', 'c1'), tag('new', 'c2')]
    sortTags(tags, commits)
    expect(names(tags)).toEqual(['old', 'new'])
  })
})
