import type { CommitNode, RefPointer } from './types'

/** 工作区伪行的 id。它不是真提交，不参与归属与装箱。 */
export const WORKDIR_ID = '__workdir__'

/** 一条「分支线」：一个 tip 加上它沿第一父链独占的提交。 */
export interface OwnerInfo {
  /** 认领顺序。0 号是主干（master/main），固定占 lane 0。 */
  index: number
  /** 该 owner 拥有的提交所占的行区间，闭区间。 */
  minRow: number
  maxRow: number
}

export interface OwnerAssignment {
  /** commit id → owner index */
  ownerOf: Map<string, number>
  owners: OwnerInfo[]
}

const MAIN_LOCAL = ['refs/heads/master', 'refs/heads/main']
const MAIN_REMOTE = ['refs/remotes/origin/master', 'refs/remotes/origin/main']

/** 不是任何提交的父 —— 也就是图里没有子节点的悬空 tip。已按行序返回。 */
function childless(commits: CommitNode[]): string[] {
  const hasChild = new Set<string>()
  for (const c of commits) for (const p of c.parents) hasChild.add(p)
  return commits.filter((c) => !hasChild.has(c.id)).map((c) => c.id)
}

/**
 * 按固定优先级挑出所有分支线的起点。
 *
 * 顺序只依赖提交拓扑与 refs，**不依赖 HEAD**：否则 checkout 换个分支
 * 整张图的车道就会重排。后端 `engine/refs.rs` 会额外塞一条 kind='head'
 * 的记录，这里必须显式跳过。
 */
export function collectTips(commits: CommitNode[], refs: RefPointer[]): string[] {
  const rowOf = new Map<string, number>()
  commits.forEach((c, i) => rowOf.set(c.id, i))

  // 指向图外提交的 ref（历史被 5000 条上限截断）认领不了，先滤掉
  const named = refs.filter((r) => r.kind !== 'head' && rowOf.has(r.hash))
  const byRow = (list: RefPointer[]) =>
    [...list].sort((a, b) => rowOf.get(a.hash)! - rowOf.get(b.hash)!).map((r) => r.hash)

  const local = named.filter((r) => r.kind === 'branch')
  const mainLocal = local.filter((r) => MAIN_LOCAL.includes(r.name))
  const mainRemote = named.filter((r) => MAIN_REMOTE.includes(r.name))

  const tiers: string[][] = [
    // 主干永远吃 lane 0；本地 master/main 优先，没有才回退远程。
    // 两者都不存在时本级为空，lane 0 自然归下一级的第一个 tip。
    byRow(mainLocal.length > 0 ? mainLocal : mainRemote),
    byRow(local),
    byRow(named.filter((r) => r.kind === 'remote')),
    byRow(named.filter((r) => r.kind === 'tag')),
    childless(commits),
  ]

  const seen = new Set<string>()
  const tips: string[] = []
  for (const tier of tiers) {
    for (const hash of tier) {
      if (seen.has(hash)) continue
      seen.add(hash)
      tips.push(hash)
    }
  }
  return tips
}

/**
 * Pass 1：把每个提交划归一条分支线。
 *
 * 按优先级从每个 tip 沿第一父链往下走，沿途还没主人的提交归该 tip；
 * 撞上已有主人的提交立即停止——那就是分叉点。
 */
export function computeOwners(commits: CommitNode[], refs: RefPointer[]): OwnerAssignment {
  const rowOf = new Map<string, number>()
  const byId = new Map<string, CommitNode>()
  commits.forEach((c, i) => {
    rowOf.set(c.id, i)
    byId.set(c.id, c)
  })

  const ownerOf = new Map<string, number>()
  const owners: OwnerInfo[] = []

  const claim = (start: string) => {
    if (ownerOf.has(start) || !rowOf.has(start)) return
    const index = owners.length
    const row = rowOf.get(start)!
    const info: OwnerInfo = { index, minRow: row, maxRow: row }
    owners.push(info)

    let cur: string | undefined = start
    while (cur && rowOf.has(cur) && !ownerOf.has(cur)) {
      ownerOf.set(cur, index)
      const r = rowOf.get(cur)!
      if (r < info.minRow) info.minRow = r
      if (r > info.maxRow) info.maxRow = r
      cur = byId.get(cur)!.parents[0]
    }

    // 这条线不止于最后一个自有提交：它还要在本车道里一直竖到分叉点（`cur`，
    // 已归别人所有的第一父）那一行，才拐进对方车道。区间漏掉这段的话，装箱
    // 会把车道判成空闲让给别人，那条竖线就从别人的圆点上穿过去——视觉上该
    // 提交凭空多出一个父节点。tip 很新而 merge base 很老的短分支必然踩中。
    if (cur !== undefined) {
      const r = rowOf.get(cur)
      if (r !== undefined && r > info.maxRow) info.maxRow = r
    }
  }

  for (const tip of collectTips(commits, refs)) claim(tip)

  // 合并提交的第二父及以后只能经由第二父边到达，上面的第一父链遍历覆盖不到。
  // 按行序补认领，保证结果与遍历顺序无关。
  for (const commit of commits) {
    const childRow = rowOf.get(commit.id)
    for (const parent of commit.parents.slice(1)) {
      claim(parent)
      // 第二父边画在**父**的车道里：从合并那行横过来，再一路竖到父那行
      // （`computePaths` 的 `cornerAtTop`）。所以父所属 owner 的车道从合并行
      // 就开始被占用，区间要往上顶到这里——和上面的出边是同一个缺陷的两面。
      // （第三面在工作区伪行上，见 `computeLayout` 里的「已知残留」。）
      const owner = ownerOf.get(parent)
      if (owner === undefined || childRow === undefined) continue
      if (childRow < owners[owner].minRow) owners[owner].minRow = childRow
    }
  }

  return { ownerOf, owners }
}
