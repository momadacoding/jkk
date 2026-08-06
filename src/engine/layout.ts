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
 * 互不重叠，会共享同一条车道。车道数只取决于**同时活跃**的分支数，
 * 而不是分支总数。
 */
export function packLanes(owners: OwnerInfo[]): number[] {
  const laneOf = new Array<number>(owners.length).fill(0)
  /** lane 序号 → 当前占用者的 maxRow；-1 表示没被用过。 */
  const laneEnd: number[] = []

  const take = (lane: number, owner: OwnerInfo) => {
    while (laneEnd.length <= lane) laneEnd.push(-1)
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
    while (lane < laneEnd.length && laneEnd[lane] >= owner.minRow) lane++
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
