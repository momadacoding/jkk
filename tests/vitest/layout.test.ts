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

  it('短分支的出边跨过的行不会被别的分支复用车道', () => {
    // 真实形状（readwise-alternative 的 worktree-ci-parallel-e2e）：
    // a1 是很新的单提交分支，但它的 merge base m2 在很下面。a1 的线要在自己
    // 的车道里从第 0 行一路竖到第 5 行才拐进 lane 0。b 分支若复用这条车道，
    // 那条竖线就从 b1/b2 的圆点上穿过去——看上去 b2 凭空多出一个父节点。
    const commits = [
      node('a1', ['m2']),
      node('m0', ['m1']),
      node('m1', ['m2']),
      node('b1', ['b2']),
      node('b2', ['m2']),
      node('m2', []),
    ]
    const refs = [
      ref('refs/heads/master', 'm0', 'branch'),
      ref('refs/heads/a', 'a1', 'branch'),
      ref('refs/heads/b', 'b1', 'branch'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect(lanes.a1).not.toBe(lanes.b1)
    expect(lanes.a1).not.toBe(lanes.b2)
  })

  it('第二父边跨过的行不会被别的分支复用车道', () => {
    // 同一个缺陷的对称面：第二父边画在**父**的车道里（`cornerAtTop`），
    // 从合并那行横过来再一路竖到父那行。父的 owner 只从自己第一个提交起算，
    // 上面这一大段就成了「空闲车道」，别的分支被塞进来后同样被线穿过。
    const commits = [
      node('c', ['m1', 'p1']),
      node('y1', ['y2']),
      node('y2', ['m1']),
      node('m1', ['m2']),
      node('m2', ['m3']),
      node('m3', ['m4']),
      node('m4', ['m5']),
      node('m5', ['m6']),
      node('m6', ['m7']),
      node('m7', ['m8']),
      node('p1', ['m8']),
      node('m8', []),
    ]
    const refs = [
      ref('refs/heads/master', 'c', 'branch'),
      ref('refs/heads/y', 'y1', 'branch'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect(lanes.p1).not.toBe(lanes.y1)
    expect(lanes.p1).not.toBe(lanes.y2)
  })

  it('线跨度不重叠的两条分支仍然共用车道', () => {
    // 上面两条把区间从「自有提交跨度」放大成「线跨度」，代价是车道数会涨。
    // 这条钉住放大的边界：跨度真的不重叠时复用必须还在，否则每条历史分支
    // 都独占一列，图会横向爆炸。
    const commits = [
      node('a1', ['m1']), // 线跨度 0..2
      node('m0', ['m1']),
      node('m1', ['m2']),
      node('m2', ['m3']),
      node('m3', ['m4']),
      node('b1', ['m5']), // 线跨度 5..7
      node('m4', ['m5']),
      node('m5', ['m6']),
      node('m6', []),
    ]
    const refs = [
      ref('refs/heads/master', 'm0', 'branch'),
      ref('refs/heads/a', 'a1', 'branch'),
      ref('refs/heads/b', 'b1', 'branch'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect(lanes.a1).toBe(lanes.b1)
    expect(lanes.a1).not.toBe(0)
  })

  it('本地 master 落后 origin/master 时，领先的那截是一条连续的线', () => {
    // 真实形状：GitHub 上连着合了两个 PR，本地 master 还停在 m。
    // master 独占 lane 0；om -> t 这截领先的主干是**一条**线，必须同车道且连续，
    // 到 m 那行才拐进 lane 0。缺陷形态是它被切成 om / t 两段两色排成阶梯：
    // 本地 master 只认领到 m 及以下，无主的 t 被普通本地分支 w1 沿第一父链吞掉，
    // origin/master 轮到时只剩 om 一个提交，另占一条车道。
    const commits = [
      node('om', ['t']), // origin/master
      node('w1', ['t']),
      node('t', ['m']),
      node('w2', ['m']),
      node('m', ['m0']), // 本地 master
      node('m0', []),
    ]
    const refs = [
      ref('refs/heads/master', 'm', 'branch'),
      ref('refs/remotes/origin/master', 'om', 'remote'),
      ref('refs/heads/w1', 'w1', 'branch'),
      ref('refs/heads/w2', 'w2', 'branch'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect(lanes.m).toBe(0)
    expect(lanes.m0).toBe(0)
    expect(lanes.om).toBe(lanes.t) // 一条线，不是两段
    expect(lanes.om).not.toBe(0)
    expect(lanes.w1).not.toBe(lanes.om)
    expect(lanes.w2).not.toBe(lanes.om)
  })

  it('骑在 origin/master 头上的分支与它同车道', () => {
    // 这是最初那张截图剩下的一段：f1 的第一父就是 om，两点必须竖直相连。
    // om 若仍作为线的起点，f1 只能另开一列，这条父子边就成了跨列斜线。
    const commits = [
      node('f1', ['om']), // 骑在 origin/master 头上的 feature
      node('om', ['t']),
      node('t', ['m']),
      node('m', []), // 本地 master
    ]
    const refs = [
      ref('refs/heads/master', 'm', 'branch'),
      ref('refs/remotes/origin/master', 'om', 'remote'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect(lanes.m).toBe(0)
    expect(lanes.f1).toBe(lanes.om)
    expect(lanes.om).toBe(lanes.t)
    expect(lanes.f1).not.toBe(0)
  })

  it('本地 master 与 origin/master 真分叉时，较新的一侧占 lane 0', () => {
    // 本地有未 push 的提交、远程也有别人推的提交，两侧互不是对方祖先。
    // 谁也不比谁更「主干」，按行序定：这里本地更新，origin/master 那截成一条支线。
    const commits = [node('lm', ['base']), node('om', ['base']), node('base', [])]
    const refs = [
      ref('refs/heads/master', 'lm', 'branch'),
      ref('refs/remotes/origin/master', 'om', 'remote'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect(lanes.lm).toBe(0)
    expect(lanes.base).toBe(0)
    expect(lanes.om).not.toBe(0)
  })

  it('没有 refs 时不崩溃，每行都有车道', () => {
    const rows = computeLayout([node('c2', ['c1']), node('c1', [])])
    expect(rows.map((r) => r.lane)).toEqual([0, 0])
  })

  it('父提交不在数据里时该父的车道退化为 0 而不是 undefined', () => {
    const rows = computeLayout([node('c1', ['truncated'])])
    expect(rows[0].parents[0].lane).toBe(0)
  })

  it('SVN 混合图里 SVN 主线整条占 lane 0，不被最上面的分支借走', () => {
    // 交付两次后再拉分支的真实形状（spliceHistory 之后）：grow 长在已交付的 mf 上，
    // feature / work 都以 r3 为第一父，keep-a 指着已交付的 a。
    // 主干只认 master/main 时，lane 0 归行序最靠前的 grow，它沿第一父链把 r2、r1
    // 认领走；r3 头上骑着 feature/work 被滤掉起点资格，又被 feature 带走——
    // SVN 主线断成两色两列。
    const commits = [
      node('g', ['mf']),
      node('f', ['svn:3']),
      node('d', ['svn:3']),
      node('mf', ['svn:2']),
      node('a', ['svn:1']),
      node('svn:3', ['svn:2']),
      node('svn:2', ['svn:1']),
      node('svn:1', []),
    ]
    const refs = [
      ref('refs/heads/grow', 'g', 'branch'),
      ref('refs/heads/feature', 'f', 'branch'),
      ref('refs/heads/work', 'd', 'branch'),
      ref('refs/heads/keep-a', 'a', 'branch'),
      ref('refs/heads/svn/base', 'svn:3', 'branch'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect([lanes['svn:3'], lanes['svn:2'], lanes['svn:1']]).toEqual([0, 0, 0])
    // 其余分支都不该挤进 lane 0
    for (const id of ['g', 'f', 'd', 'mf', 'a']) expect(lanes[id]).not.toBe(0)
  })

  it('比基线新的 SVN revision 也在 lane 0，主干从 SVN 链顶端算起而不是 svn/base', () => {
    // 同事提交了 r4、本地还没更新：svn/base 仍在 r3，r4 在它上面
    const commits = [
      node('svn:4', ['svn:3']),
      node('d', ['svn:3']),
      node('svn:3', ['svn:2']),
      node('svn:2', []),
    ]
    const refs = [
      ref('refs/heads/work', 'd', 'branch'),
      ref('refs/heads/svn/base', 'svn:3', 'branch'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect([lanes['svn:4'], lanes['svn:3'], lanes['svn:2']]).toEqual([0, 0, 0])
    expect(lanes.d).not.toBe(0)
  })

  it('普通 git 仓库里叫 svn/base 的分支不会抢 master 的 lane 0', () => {
    const commits = [node('b1', ['m1']), node('m1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/svn/base', 'b1', 'branch'),
      ref('refs/heads/master', 'm1', 'branch'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect(lanes.m1).toBe(0)
    expect(lanes.b1).not.toBe(0)
  })
})
