import { describe, it, expect } from 'vitest'
import { computeLayout } from '../../src/engine/layout'
import { computePaths } from '../../src/engine/paths'
import type { CommitNode } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

describe('computePaths', () => {
  it('creates one segment per parent link', () => {
    const commits = [node('c2', ['c1']), node('c1', [])]
    const rows = computeLayout(commits)
    const paths = computePaths(rows, 28, 20)
    expect(paths.length).toBe(1)
    expect(paths[0].segments.length).toBe(1)
  })

  it('uses different paths for merge lanes', () => {
    const commits = [node('c4', ['c3', 'c2']), node('c3', ['c1']), node('c2', ['c1']), node('c1', [])]
    const rows = computeLayout(commits)
    const paths = computePaths(rows, 28, 20)
    const mergePath = paths.find((p) => p.segments.some((s) => s.curve))
    expect(mergePath).toBeDefined()
  })
})
