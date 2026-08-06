import { describe, it, expect } from 'vitest'
import { computeLayout } from '../../src/engine/layout'
import { LANE_PALETTE, computePaths, filterPathsToBand, laneColor } from '../../src/engine/paths'
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

  it('colors are stable per lane across input slices', () => {
    // 分叉历史：lane 0（master 主干）+ lane 1（分支）
    const commits = [node('c4', ['c3', 'c2']), node('c3', ['c1']), node('c2', ['c1']), node('c1', [])]
    const allRows = computeLayout(commits)
    const full = computePaths(allRows, 28, 20)
    // 切片只含 lane-1（c2→c1）。曾经的实现按插入顺序取色，
    // 这个切片会把 lane-1 画成 PALETTE[0]，与全量视图颜色不一致
    const slice = computePaths(allRows.slice(2, 4), 28, 20)

    const fullLane1 = full.find((p) => p.id === 'lane-1')
    const sliceLane1 = slice.find((p) => p.id === 'lane-1')
    expect(fullLane1).toBeDefined()
    expect(sliceLane1).toBeDefined()
    expect(sliceLane1!.color).toBe(fullLane1!.color)
    expect(sliceLane1!.color).toBe(laneColor(1))
    expect(fullLane1!.color).toBe(LANE_PALETTE[1 % LANE_PALETTE.length])
  })

  it('keeps segments for parents far below the band', () => {
    // 100 个 commit 的线性链 → 99 条线段，一条都不能丢
    const commits = Array.from({ length: 100 }, (_, i) => {
      const id = `c${99 - i}`
      return node(id, i === 99 ? [] : [`c${98 - i}`])
    })
    const rows = computeLayout(commits)
    const paths = computePaths(rows, 28, 20)
    expect(paths.find((p) => p.id === 'lane-0')?.segments.length).toBe(99)
  })

  it('connects every segment to where its parent actually sits', () => {
    // 还原真实仓库的形状：merge 的第一父 c1 是主干、第二父 c3 是被并入的分支，
    // 而 c3 这条线最老的提交 c2 的父又是 c1。
    // c1 在 merge 那一步被预留到 lane 0，所以 c2 的 LayoutParent.lane（预留值）
    // 和 c1 实际渲染的 lane 不一致——用预留值就会画出一段接不到圆点的悬空竖线。
    const commits = [
      node('merge', ['c1', 'c3']),
      node('c3', ['c2']),
      node('c2', ['c1']),
      node('c1', []),
    ]
    const rows = computeLayout(commits)
    const byId = Object.fromEntries(rows.map((r) => [r.commit.id, r]))
    const paths = computePaths(rows, 28, 20)
    const centre = (lane: number) => lane * 20 + 10
    const midY = (row: number) => row * 28 + 14

    const endpoints = paths.flatMap((p) => p.segments.map((s) => `${s.x1},${s.y1}->${s.x2},${s.y2}`))
    // c2 → c1 必须真的落在 c1 的圆点上
    const expected = `${centre(byId.c2.lane)},${midY(byId.c2.row)}->${centre(byId.c1.lane)},${midY(byId.c1.row)}`
    expect(endpoints).toContain(expected)

    // 每条线段的终点都必须正好是某个提交圆点的位置，不能悬空
    const dots = new Set(rows.map((r) => `${centre(r.lane)},${midY(r.row)}`))
    for (const p of paths) {
      for (const s of p.segments) {
        expect(dots.has(`${s.x1},${s.y1}`)).toBe(true)
        expect(dots.has(`${s.x2},${s.y2}`)).toBe(true)
      }
    }
  })

  it('drops segments whose parent commit is missing from input', () => {
    // 5000 条上限截断时父节点不在数据里，画不了就跳过而不是崩溃
    const rows = computeLayout([node('c2', ['missing'])])
    const paths = computePaths(rows, 28, 20)
    expect(paths).toHaveLength(0)
  })

  it('第一父的跨车道边拐角在下，合并第二父的拐角在上', () => {
    // 线段要笔直地待在它自己所属的车道里，只在另一端拐一次。
    // 第一父的线归子节点的车道 → 竖到父节点那行才拐（cornerAtTop=false）；
    // 第二父的线归父节点的车道 → 从子节点横出去后立刻竖下去（cornerAtTop=true）。
    const commits = [node('merge', ['m1', 'b1']), node('b1', ['m0']), node('m1', ['m0']), node('m0', [])]
    const rows = computeLayout(commits)
    const byId = Object.fromEntries(rows.map((r) => [r.commit.id, r]))
    const paths = computePaths(rows, 28, 20)
    const segs = paths.flatMap((p) => p.segments)
    const at = (id: string) => ({ x: byId[id].lane * 20 + 10, y: byId[id].row * 28 + 14 })

    const find = (from: string, to: string) => {
      const a = at(from), b = at(to)
      return segs.find((s) => s.x1 === a.x && s.y1 === a.y && s.x2 === b.x && s.y2 === b.y)
    }

    // merge 的第二父 b1 落在另一条车道上，这条边归 b1 的车道
    const second = find('merge', 'b1')
    expect(second).toBeDefined()
    expect(second!.curve).toBe(true)
    expect(second!.cornerAtTop).toBe(true)

    // 同车道的边不拐弯，直接是直线
    const first = find('merge', 'm1')
    expect(first).toBeDefined()
    expect(first!.cornerAtTop).toBe(false)
  })

  it('跨车道的第一父边把拐角放在下端', () => {
    // 真实仓库里 Merge PR #1 → master 跨了 16 行，正是这条路径
    const commits = [node('tip', ['m1']), node('side', ['m1']), node('m1', [])]
    const rows = computeLayout(commits)
    const byId = Object.fromEntries(rows.map((r) => [r.commit.id, r]))
    // side 与 m1 不同车道时才构成跨车道的第一父边
    const paths = computePaths(rows, 28, 20)
    const seg = paths
      .flatMap((p) => p.segments)
      .find((s) => s.x1 === byId.side.lane * 20 + 10 && s.x2 === byId.m1.lane * 20 + 10 && s.curve)
    expect(seg).toBeDefined()
    expect(seg!.cornerAtTop).toBe(false)
  })
})

describe('filterPathsToBand', () => {
  it('keeps segments spanning the band boundary', () => {
    const paths = [{
      id: 'lane-0',
      color: '#fff',
      segments: [{ x1: 10, y1: 500, x2: 10, y2: 3000 }], // 从条带上沿穿过到下沿外
    }]
    const out = filterPathsToBand(paths, 560, 1120)
    expect(out[0].segments).toHaveLength(1)
  })

  it('drops segments fully above or below the band', () => {
    const paths = [{
      id: 'lane-0',
      color: '#fff',
      segments: [
        { x1: 0, y1: 100, x2: 0, y2: 200 },   // 完全在条带上方
        { x1: 0, y1: 5000, x2: 0, y2: 6000 }, // 完全在条带下方
      ],
    }]
    const out = filterPathsToBand(paths, 560, 1120)
    expect(out).toHaveLength(0)
  })

  it('drops lanes that have no visible segments left', () => {
    const paths = [
      { id: 'lane-0', color: '#fff', segments: [{ x1: 0, y1: 100, x2: 0, y2: 200 }] },
      { id: 'lane-1', color: '#000', segments: [{ x1: 0, y1: 600, x2: 0, y2: 700 }] },
    ]
    const out = filterPathsToBand(paths, 560, 1120)
    expect(out.map((p) => p.id)).toEqual(['lane-1'])
  })
})
