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

  // 本地分支默认按 tip 行序认领，谁的 tip 新谁先拿走共享的那段历史。两条分支共享一段不在
  // 主干上的提交时，这就把「后切出来、但刚提交过」的分支画成了原分支。reflog 证据能纠正它。
  tiers[2] = orderByMadeHere(tiers[2], local, commits, [...tiers[0], ...tiers[1]])

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

/** 从这些起点沿第一父链能走到的所有提交（只算图里有的）。 */
function firstParentChains(commits: CommitNode[], starts: string[]): Set<string> {
  const byId = new Map(commits.map((c) => [c.id, c]))
  const reached = new Set<string>()
  for (const start of starts) {
    let cur: string | undefined = start
    while (cur && byId.has(cur) && !reached.has(cur)) {
      reached.add(cur)
      cur = byId.get(cur)!.parents[0]
    }
  }
  return reached
}

/**
 * 按 reflog 证据调整本地分支的认领顺序；没有证据时原样返回（tip 行序）。
 *
 * git 不记录提交是在哪条分支上做的，后端从本机 reflog 读出每条本地分支「在这里新建过」的
 * 提交（`RefPointer.madeHere`）。两条分支的第一父链在主干之外汇合时，汇合处那个提交（分叉点）
 * 是在哪条分支上做的，那条分支就是原分支，排到另一条前面，先把共享的那段认领走。
 *
 * 只看分叉点、不看更老的共享提交：分叉点之下的历史两边都有，更老的证据说明不了是谁从谁切出来的。
 * 证据有矛盾（成环）时按 tip 行序打破，所以最坏也只是退回原来的画法。
 *
 * 不依赖 HEAD，checkout 不会让图重排；reflog 有新记录时图才会变。
 */
function orderByMadeHere(
  tips: string[],
  local: RefPointer[],
  commits: CommitNode[],
  trunkTips: string[],
): string[] {
  // 同一个提交上挂着两个本地分支时 hash 会重复，后面按 hash 去重，这里先去掉
  const unique = [...new Set(tips)]
  const tipSet = new Set(unique)
  /** 提交 → 在它上面新建过它的那些分支的 tip */
  const madeOn = new Map<string, Set<string>>()
  for (const r of local) {
    if (!tipSet.has(r.hash)) continue
    for (const oid of r.madeHere ?? []) {
      let set = madeOn.get(oid)
      if (!set) madeOn.set(oid, (set = new Set()))
      set.add(r.hash)
    }
  }
  if (madeOn.size === 0) return tips

  const byId = new Map<string, CommitNode>()
  const rowOf = new Map<string, number>()
  /** 以它为第一父的子提交个数 */
  const firstParentChildren = new Map<string, number>()
  commits.forEach((c, i) => {
    byId.set(c.id, c)
    rowOf.set(c.id, i)
    const p = c.parents[0]
    if (p) firstParentChildren.set(p, (firstParentChildren.get(p) ?? 0) + 1)
  })
  // 两条链从不同的子走进分叉点，所以分叉点至少有两个以它为第一父的子。别的证据用不上，先扔掉：
  // 分支自己的提交大多是单链，这一步通常把证据砍到只剩几个。
  for (const oid of [...madeOn.keys()]) {
    if ((firstParentChildren.get(oid) ?? 0) < 2) madeOn.delete(oid)
  }
  // 行序里比最老的候选分叉点还靠下的提交不可能是分叉点，链走到那里就停。没有主干（develop、
  // trunk 这类仓库）时链本来会一路走到窗口底部，这条界限让开销只跟证据的跨度有关。
  let lastEvidenceRow = -1
  for (const oid of madeOn.keys()) lastEvidenceRow = Math.max(lastEvidenceRow, rowOf.get(oid) ?? -1)
  if (lastEvidenceRow < 0) return tips

  // 主干先于所有本地分支认领，它的第一父链不参与比较：证据抢不走主干上的提交
  const trunkClaimed = firstParentChains(commits, trunkTips)
  const chainOf = new Map<string, string[]>()
  for (const tip of unique) {
    const chain: string[] = []
    let cur: string | undefined = tip
    while (cur && byId.has(cur) && !trunkClaimed.has(cur) && rowOf.get(cur)! <= lastEvidenceRow) {
      chain.push(cur)
      cur = byId.get(cur)!.parents[0]
    }
    chainOf.set(tip, chain)
  }
  // 只有做过证据提交的分支才可能被排到别人前面，只给它们建查找集合
  const chainSetOf = new Map<string, Set<string>>()
  for (const makers of madeOn.values()) {
    for (const maker of makers) {
      if (!chainSetOf.has(maker)) chainSetOf.set(maker, new Set(chainOf.get(maker)))
    }
  }

  /** tip → 必须排在它前面的 tip */
  const before = new Map<string, Set<string>>()
  for (const tip of unique) {
    const chain = chainOf.get(tip)!
    chain.forEach((oid, i) => {
      const makers = madeOn.get(oid)
      if (!makers || makers.has(tip)) return
      for (const maker of makers) {
        const theirs = chainSetOf.get(maker)!
        // 第一父链汇合之后就一直重合。oid 在对方链上、而 tip 走到 oid 之前的那个提交不在，
        // 说明两条链在 oid 第一次汇合：oid 是两者的分叉点
        if (!theirs.has(oid) || (i > 0 && theirs.has(chain[i - 1]))) continue
        let set = before.get(tip)
        if (!set) before.set(tip, (set = new Set()))
        set.add(maker)
      }
    })
  }

  const placed = new Set<string>()
  const ordered: string[] = []
  const remaining = [...unique]
  while (remaining.length > 0) {
    let pick = remaining.findIndex((t) => [...(before.get(t) ?? [])].every((u) => placed.has(u)))
    if (pick < 0) pick = 0 // 成环：按行序打破
    const [tip] = remaining.splice(pick, 1)
    placed.add(tip)
    ordered.push(tip)
  }
  return ordered
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
