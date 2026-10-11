import type { CommitNode, RefPointer } from './types'

/** 工作区伪行的 id。它不是真提交，不参与归属与装箱。 */
export const WORKDIR_ID = '__workdir__'

/** 一条「分支线」：一个 tip 加上它沿第一父链独占的提交。 */
export interface OwnerInfo {
  /** 认领顺序。0 号是主干（SVN 链，否则 master/main），固定占 lane 0。 */
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

  // 有子以它为第一父的提交**不独立成线**：那个子的线沿第一父链下来必然经过它，
  // 让它另起一条线只会把一条本该竖直的父子边劈成跨列斜线。
  //
  // 这是「主干被切断」的真正根因：owner 只能从起点沿第一父链往**下**走，永远走不到
  // 自己的子。所以只要有分支骑在某条线的起点头上（feature 骑在 origin/master 上、
  // origin/master 骑在落后的 master 上），那条线就在起点处断开。收紧起点的定义之后，
  // 骑在头上的那个子会把下面整条链一起认领走，父子边自然竖直。Sublime Merge 同此。
  const hasFirstParentChild = new Set<string>()
  for (const c of commits) if (c.parents[0]) hasFirstParentChild.add(c.parents[0])

  // lane 0 的专座，也是上面那条规则的**唯一例外**：主干必须强制成为起点，哪怕有分支
  // 骑在它头上。否则 master 会被那条分支的线吞掉，失去自己的车道——而 master 的位置
  // 固定在最左列是这张图的锚点，不该随便哪条分支的存在与否而漂移。
  // 本地主干优先，没有才回退远程主干；两者都没有时本级为空，lane 0 归下一级第一个 tip。
  //
  // SVN 混合图里主干是 SVN 链本身，压过 master/main：它的顶端（行序最靠前的 `svn:N`）
  // 起线，沿第一父链一路认领到最老的 revision。SVN 仓库没有 master，这一级为空时
  // lane 0 会归行序最靠前的随便哪条分支，它沿第一父链把下面几个 revision 认领走，
  // 剩下的 revision 又被另一条分支带走——SVN 主线断成两列两色。
  // 不认 `svn/base`：比基线新、还没更新的同事 revision 在它上面，会被甩出主线；
  // 而且普通 git 仓库里叫 svn/base 的分支是用户自己的。`svn:` 节点只有 `spliceHistory`
  // 接合过的图才有，所以这条只在 SVN 模式下生效。
  const svnTop = commits.find((c) => c.id.startsWith('svn:'))?.id
  const trunk = svnTop ? [svnTop] : byRow(mainLocal.length > 0 ? mainLocal : mainRemote)

  const tiers: string[][] = [
    trunk,
    // 远程主干排在**普通本地分支之前**。在 GitHub 上合完 PR、本地还没 pull 时，本地
    // master 落后于 origin/master 是常态而非异常；远程主干若排在普通本地分支之后，
    // 本地 master 之上那几个提交在轮到它之前就无主，会被随便哪条本地分支沿第一父链
    // 吞掉，主干被切成几段几色，图上排成阶梯。
    //
    // 注意这一级只在 origin/master 自己就是线起点时才生效——有分支骑在它头上时它已被
    // 上面的规则滤掉，由那条分支的线连着一起认领，效果一样是主干不断。
    byRow(mainRemote),
    byRow(local),
    byRow(named.filter((r) => r.kind === 'remote')),
    byRow(named.filter((r) => r.kind === 'tag')),
    childless(commits),
  ].map((tier, i) => (i === 0 ? tier : tier.filter((h) => !hasFirstParentChild.has(h))))

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
