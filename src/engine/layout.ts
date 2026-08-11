import type { CommitNode, RefPointer } from './types'
import type { LayoutNode } from './graphTypes'
import { computeOwners, WORKDIR_ID, type OwnerInfo } from './owners'

/**
 * Pass 2：把每个 owner 映射到一条车道。
 *
 * owner 0（主干）固定 lane 0 且不被复用；其余按 minRow 升序，
 * 各自挑第一条「当前占用者已经结束」的车道。
 *
 * 车道复用是这个设计能用的前提：仓库里几十个早已合并的旧分支行区间
 * 互不重叠，会共享同一条车道。车道数只取决于**行区间重叠**的分支数，
 * 而不是分支总数。
 *
 * 注意区间是「线的跨度」而不是「自有提交的跨度」（见 `computeOwners`）：
 * 一条分支从 tip 一直占到它的分叉点那行。所以 tip 很新、分叉点很老的
 * 长命分支会占住一大段车道，哪怕它只有一个提交。20 个这样的分支就是
 * 20 条车道——这是正确的，它们的线在图上确实同时并行；但代价是列数会
 * 比只按提交行数算时多。真要压缩只能改线的走法，不能改区间。
 */
/**
 * 复用车道前要求的垂直间隔（行）。
 *
 * 上一个占用者刚结束、下一条分支紧接着开始时，两段线在同一列里首尾相接，
 * 看上去就是一条连续的线——合并点因此从视觉上消失。留出间隔可以避免这种
 * 「假连续」。间隔不够就另开一列。
 */
const LANE_REUSE_GAP = 2

export function packLanes(owners: OwnerInfo[]): number[] {
  const laneOf = new Array<number>(owners.length).fill(0)
  /** lane 序号 → 当前占用者的 maxRow；-Infinity 表示没被用过。 */
  const laneEnd: number[] = []

  const take = (lane: number, owner: OwnerInfo) => {
    while (laneEnd.length <= lane) laneEnd.push(Number.NEGATIVE_INFINITY)
    laneEnd[lane] = owner.maxRow
    laneOf[owner.index] = lane
  }

  const order = [...owners].sort((a, b) => a.minRow - b.minRow || a.index - b.index)
  for (const owner of order) {
    if (owner.index === 0) {
      take(0, owner)
      continue
    }
    // 从 1 开始找：lane 0 是主干的专座
    let lane = 1
    while (lane < laneEnd.length && laneEnd[lane] >= owner.minRow - LANE_REUSE_GAP) lane++
    take(lane, owner)
  }
  return laneOf
}

/**
 * 两趟布局：先算归属（`computeOwners`），再把 owner 装箱到车道。
 *
 * `refs` 可省略，省略时退化成「按悬空 tip 分线」——纯拓扑视图，
 * 既有的 paths 测试就靠这条路径。
 */
export function computeLayout(commits: CommitNode[], refs: RefPointer[] = []): LayoutNode[] {
  // 工作区伪行不参与归属与装箱。让它参与的话，HEAD 在哪个分支就会改变
  // 该分支 owner 的行区间，进而挤动别人的车道——图就跟着 checkout 变了。
  //
  // 已知残留：正因为不参与，伪行连向 HEAD 的那条虚线也没被算进任何车道区间。
  // HEAD 落在一条复用了别人车道的分支上时，这条虚线会从中间那些提交的圆点上
  // 穿过去，症状和 `computeOwners` 里修掉的那两种一样。这里是刻意不修——
  // 修了就得让伪行参与装箱，图会随 checkout 重排，代价比这条虚线大。
  const real = commits.filter((c) => c.id !== WORKDIR_ID)
  const { ownerOf, owners } = computeOwners(real, refs)
  const laneOfOwner = packLanes(owners)

  const laneOfCommit = new Map<string, number>()
  for (const [id, owner] of ownerOf) laneOfCommit.set(id, laneOfOwner[owner] ?? 0)
  const laneOf = (id: string) => laneOfCommit.get(id) ?? 0

  return commits.map((commit, row) => ({
    commit,
    row,
    // 伪行直接取父节点的车道，见上面的注释
    lane: commit.id === WORKDIR_ID ? laneOf(commit.parents[0] ?? '') : laneOf(commit.id),
    parents: commit.parents.map((hash, idx) => ({
      hash,
      lane: laneOf(hash),
      isMain: idx === 0,
    })),
  }))
}
