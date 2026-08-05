import { describe, it, expect } from 'vitest'
import { computeLayout } from '../../src/engine/layout'
import type { CommitNode } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

describe('computeLayout', () => {
  it('lays out linear history on single lane', () => {
    const commits = [node('c3', ['c2']), node('c2', ['c1']), node('c1', [])]
    const layout = computeLayout(commits)
    expect(layout.every((n) => n.lane === 0)).toBe(true)
    expect(layout[0].parents[0].lane).toBe(0)
  })

  it('puts branch on new lane', () => {
    // c3 -> c1, c2 -> c1
    const commits = [node('c3', ['c1']), node('c2', ['c1']), node('c1', [])]
    const layout = computeLayout(commits)
    expect(layout[0].lane).not.toBe(layout[1].lane)
    expect(layout[2].lane).toBe(0)
  })

  it('assigns secondary parent to own lane on merge', () => {
    // c4 merges c3 and c2; both point to c1
    const commits = [node('c4', ['c3', 'c2']), node('c3', ['c1']), node('c2', ['c1']), node('c1', [])]
    const layout = computeLayout(commits)
    const merge = layout[0]
    expect(merge.parents[0].lane).toBe(merge.lane)
    expect(merge.parents[1].lane).not.toBe(merge.lane)
  })
})
