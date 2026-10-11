// @vitest-environment jsdom
// 只断言节点画成什么形状；形状好不好认仍要在真实窗口里看（AGENTS.md §6.1）。
import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { createSvgRenderer } from '../../src/engine/svgRenderer'
import type { GraphLayout, LayoutNode } from '../../src/engine/graphTypes'
import type { CommitNode, RefPointer } from '../../src/engine/types'

function node(commit: Partial<CommitNode> & { id: string }, row: number, lane = 0): LayoutNode {
  return {
    commit: { parents: [], author: 'a', time: 0, message: commit.id, ...commit },
    row,
    lane,
    parents: [],
  }
}

function draw(rows: LayoutNode[], refs: RefPointer[] = [], selectedId: string | null = null) {
  const layout: GraphLayout = {
    rows,
    paths: [],
    laneCount: 2,
    rowHeight: 24,
    laneWidth: 16,
    offsetY: 0,
  }
  const { container } = render(<>{createSvgRenderer().render(layout, refs, selectedId)}</>)
  return (id: string) => container.querySelector(`[data-id="${id}"]`)!
}

describe('svgRenderer 节点形状', () => {
  it('svn 节点画成菱形，git 提交仍是实心圆', () => {
    const at = draw([
      node({ id: 'g1', vcs: 'git' }, 0, 1),
      node({ id: 'svn:3', vcs: 'svn', svnRev: 3 }, 1),
    ])
    expect(at('svn:3').querySelector('circle')).toBeNull()
    expect(at('svn:3').querySelector('polygon')).not.toBeNull()
    expect(at('svn:3').getAttribute('class')).toContain('svn')
    expect(at('g1').querySelector('circle')).not.toBeNull()
    expect(at('g1').querySelector('polygon')).toBeNull()
  })

  it('菱形以车道中心为中心、四个顶点上下左右对称', () => {
    const at = draw([node({ id: 'svn:2', vcs: 'svn', svnRev: 2 }, 1, 1)])
    // lane 1 的中心 x = 16 + 8 = 24，row 1 的中心 y = 24 + 12 = 36
    const pts = at('svn:2').querySelector('polygon')!.getAttribute('points')!
      .trim().split(/\s+/).map((p) => p.split(',').map(Number))
    expect(pts).toHaveLength(4)
    const xs = pts.map(([x]) => x)
    const ys = pts.map(([, y]) => y)
    expect((Math.min(...xs) + Math.max(...xs)) / 2).toBe(24)
    expect((Math.min(...ys) + Math.max(...ys)) / 2).toBe(36)
  })

  it('HEAD 落在 svn 节点上（checkout 到 svn/base）时菱形同样放大、描边加粗', () => {
    const head: RefPointer = { name: 'HEAD', hash: 'svn:3', kind: 'head' }
    const half = (el: Element) => {
      const ys = el.querySelector('polygon')!.getAttribute('points')!
        .trim().split(/\s+/).map((p) => Number(p.split(',')[1]))
      return (Math.max(...ys) - Math.min(...ys)) / 2
    }
    const plain = draw([node({ id: 'svn:3', vcs: 'svn', svnRev: 3 }, 0)])('svn:3')
    const atHead = draw([node({ id: 'svn:3', vcs: 'svn', svnRev: 3 }, 0)], [head])('svn:3')
    expect(half(atHead)).toBeGreaterThan(half(plain))
    const width = (el: Element) => Number(el.querySelector('polygon')!.getAttribute('stroke-width'))
    expect(width(atHead)).toBeGreaterThan(width(plain))
  })

  it('选中的 svn 节点描边加粗，和 git 提交的选中一致', () => {
    const plain = draw([node({ id: 'svn:1', vcs: 'svn', svnRev: 1 }, 0)])('svn:1')
    const picked = draw([node({ id: 'svn:1', vcs: 'svn', svnRev: 1 }, 0)], [], 'svn:1')('svn:1')
    const width = (el: Element) => Number(el.querySelector('polygon')!.getAttribute('stroke-width'))
    expect(width(picked)).toBeGreaterThan(width(plain))
    expect(picked.getAttribute('class')).toContain('selected')
  })
})
