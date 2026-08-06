import { describe, it, expect } from 'vitest'
import { computeLayout, packLanes } from '../../src/engine/layout'
import type { CommitNode, RefPointer } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

function ref(name: string, hash: string, kind: RefPointer['kind']): RefPointer {
  return { name, hash, kind }
}

function laneById(rows: ReturnType<typeof computeLayout>): Record<string, number> {
  return Object.fromEntries(rows.map((r) => [r.commit.id, r.lane]))
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

describe('packLanes', () => {
  it('owner 0 固定拿 lane 0', () => {
    expect(packLanes([{ index: 0, minRow: 0, maxRow: 5 }])).toEqual([0])
  })

  it('行区间不重叠的两个 owner 复用同一条车道', () => {
    // 这条是「20 个分支不会变成 20 列」的唯一依据
    const lanes = packLanes([
      { index: 0, minRow: 0, maxRow: 20 }, // 主干
      { index: 1, minRow: 1, maxRow: 3 },
      { index: 2, minRow: 8, maxRow: 10 }, // 与 owner 1 完全不重叠
    ])
    expect(lanes[0]).toBe(0)
    expect(lanes[1]).toBe(lanes[2])
    expect(lanes[1]).not.toBe(0)
  })

  it('行区间重叠的两个 owner 必须占不同车道', () => {
    const lanes = packLanes([
      { index: 0, minRow: 0, maxRow: 20 },
      { index: 1, minRow: 1, maxRow: 9 },
      { index: 2, minRow: 5, maxRow: 12 },
    ])
    expect(new Set([lanes[0], lanes[1], lanes[2]]).size).toBe(3)
  })

  it('紧挨着的两个 owner 不复用同一条车道', () => {
    // owner 1 到第 13 行结束、owner 2 从第 14 行开始时，若复用同一列，
    // 两段线会首尾相接成一条「假连续」的线，合并点从视觉上消失。
    // 这正是真实仓库里 feat-mvp 看不出何时合并进 master 的原因。
    const lanes = packLanes([
      { index: 0, minRow: 0, maxRow: 30 },
      { index: 1, minRow: 1, maxRow: 13 },
      { index: 2, minRow: 14, maxRow: 25 },
    ])
    expect(lanes[1]).not.toBe(lanes[2])
  })

  it('相隔足够远的 owner 仍然复用车道', () => {
    // 间隔机制不能把复用彻底关掉，否则分支一多车道就爆炸
    const lanes = packLanes([
      { index: 0, minRow: 0, maxRow: 30 },
      { index: 1, minRow: 1, maxRow: 3 },
      { index: 2, minRow: 10, maxRow: 15 },
    ])
    expect(lanes[1]).toBe(lanes[2])
  })

  it('第一条分支车道从 0 行开始也不会被误判为已占用', () => {
    // laneEnd 的初值若用 -1，minRow=0 时 -1 >= 0-2 成立，空车道会被当成占用
    const lanes = packLanes([
      { index: 0, minRow: 5, maxRow: 30 },
      { index: 1, minRow: 0, maxRow: 4 },
    ])
    expect(lanes[1]).toBe(1)
  })

  it('主干的车道不被别的 owner 复用', () => {
    // owner 0 早早结束，但 lane 0 仍然只属于它
    const lanes = packLanes([
      { index: 0, minRow: 0, maxRow: 1 },
      { index: 1, minRow: 5, maxRow: 9 },
    ])
    expect(lanes[0]).toBe(0)
    expect(lanes[1]).not.toBe(0)
  })
})

describe('computeLayout 按分支归属分车道', () => {
  it('骑在 master 头上的分支 tip 占自己的车道，master 在 lane 0', () => {
    // 这就是截图里的形状：feat 分支只比 master 多一个提交
    const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect(lanes.m1).toBe(0)
    expect(lanes.m0).toBe(0)
    expect(lanes.f1).not.toBe(0)
  })

  it('切换 HEAD 不改变任何车道', () => {
    // 图形只由拓扑与 refs 决定，见规格「设计原则」
    const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
    const base = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const onMaster = computeLayout(commits, [...base, ref('HEAD', 'm1', 'head')])
    const onFeature = computeLayout(commits, [...base, ref('HEAD', 'f1', 'head')])
    expect(laneById(onFeature)).toEqual(laneById(onMaster))
  })

  it('LayoutParent.lane 等于父提交实际落到的车道', () => {
    // 上一轮修过的悬空竖线就是因为这里存的是预留值而非实际值
    const commits = [node('f1', ['m1']), node('m1', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const rows = computeLayout(commits, refs)
    const byId = Object.fromEntries(rows.map((r) => [r.commit.id, r]))
    expect(byId.f1.parents[0].lane).toBe(byId.m1.lane)
  })

  it('没有 refs 时不崩溃，每行都有车道', () => {
    const rows = computeLayout([node('c2', ['c1']), node('c1', [])])
    expect(rows.map((r) => r.lane)).toEqual([0, 0])
  })

  it('父提交不在数据里时该父的车道退化为 0 而不是 undefined', () => {
    const rows = computeLayout([node('c1', ['truncated'])])
    expect(rows[0].parents[0].lane).toBe(0)
  })
})
