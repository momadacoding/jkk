import { describe, expect, it } from 'vitest'
import { buildRefPills, shortRefName } from '../../src/engine/refPills'
import type { RefPointer } from '../../src/engine/types'

const HASH = 'a'.repeat(40)
const ref = (name: string, kind: RefPointer['kind']): RefPointer => ({
  name,
  hash: HASH,
  kind,
  ahead: null,
  behind: null,
})
const local = (name: string) => ref(`refs/heads/${name}`, 'branch')
const remote = (name: string) => ref(`refs/remotes/${name}`, 'remote')
const tag = (name: string) => ref(`refs/tags/${name}`, 'tag')

describe('shortRefName', () => {
  it('去掉三种前缀', () => {
    expect(shortRefName('refs/heads/feat/x')).toBe('feat/x')
    expect(shortRefName('refs/remotes/origin/feat/x')).toBe('origin/feat/x')
    expect(shortRefName('refs/tags/v1.0')).toBe('v1.0')
  })

  it('只剥开头的前缀，名字中间出现的同样字样保留', () => {
    expect(shortRefName('refs/heads/refs/heads/x')).toBe('refs/heads/x')
  })
})

describe('buildRefPills', () => {
  it('空输入给出空列表', () => {
    expect(buildRefPills([])).toEqual([])
  })

  it('同一行上的本地分支和同名远程分支合成一个标签', () => {
    const pills = buildRefPills([
      local('worktree-fix-delete-account-haspassword-fallback'),
      remote('origin/worktree-fix-delete-account-haspassword-fallback'),
    ])
    expect(pills).toHaveLength(1)
    expect(pills[0].kind).toBe('branch')
    expect(pills[0].label).toBe('worktree-fix-delete-account-haspassword-fallback')
    expect(pills[0].remotes).toEqual(['origin/worktree-fix-delete-account-haspassword-fallback'])
    // PR 查找和 merged-ahead 判断仍然拿本地分支
    expect(pills[0].ref.name).toBe('refs/heads/worktree-fix-delete-account-haspassword-fallback')
  })

  it('远程分支排在本地分支前面也能合并，且位置跟本地分支走', () => {
    const pills = buildRefPills([remote('origin/main'), tag('v1'), local('main')])
    expect(pills.map((p) => p.label)).toEqual(['v1', 'main'])
    expect(pills[1].remotes).toEqual(['origin/main'])
  })

  it('多个远程上的同名分支都并进同一个标签', () => {
    const pills = buildRefPills([local('main'), remote('origin/main'), remote('upstream/main')])
    expect(pills).toHaveLength(1)
    expect(pills[0].remotes).toEqual(['origin/main', 'upstream/main'])
  })

  it('名字带斜杠的分支按完整名匹配', () => {
    const pills = buildRefPills([local('feat/x'), remote('origin/feat/x')])
    expect(pills).toHaveLength(1)
    expect(pills[0].remotes).toEqual(['origin/feat/x'])
  })

  it('只是后缀相同的远程分支不合并', () => {
    // origin/feat/x 对应的分支是 feat/x，不是 x
    const pills = buildRefPills([local('x'), remote('origin/feat/x')])
    expect(pills.map((p) => p.label)).toEqual(['x', 'origin/feat/x'])
    expect(pills[0].remotes).toEqual([])
  })

  it('没有同名本地分支的远程分支照常单独显示', () => {
    const pills = buildRefPills([local('main'), remote('origin/other')])
    expect(pills.map((p) => [p.kind, p.label])).toEqual([
      ['branch', 'main'],
      ['remote', 'origin/other'],
    ])
  })

  it('大小写不同不算同名：git ref 区分大小写', () => {
    const pills = buildRefPills([local('Main'), remote('origin/main')])
    expect(pills).toHaveLength(2)
  })

  it('同名 tag 不会被当成远程分支并进来', () => {
    const pills = buildRefPills([local('v1'), tag('origin/v1')])
    expect(pills).toHaveLength(2)
    expect(pills[0].remotes).toEqual([])
  })

  it('名字里没有斜杠的远程 ref 不参与合并也不丢', () => {
    const pills = buildRefPills([local('main'), remote('main')])
    expect(pills).toHaveLength(2)
  })

  it('每个标签的 key 唯一且就是 ref 全名', () => {
    const pills = buildRefPills([local('a'), remote('origin/a'), local('b'), tag('b')])
    const keys = pills.map((p) => p.key)
    expect(new Set(keys).size).toBe(keys.length)
    expect(keys).toEqual(['refs/heads/a', 'refs/heads/b', 'refs/tags/b'])
  })
})
