/**
 * 混合图：把 SVN 历史与本地 git 历史接成一张图（spec §10.3）。
 *
 * 这一层是**纯函数**：无 React、无 Tauri 依赖，可直接单测（`AGENTS.md` §7.2）。
 * `engine/layout.ts` 与 `engine/svgRenderer.tsx` 只认 `parents`（spec §10.4），
 * 唯一的例外在 `engine/owners.ts`：主干要认 SVN 链的顶端（`svn:` 节点），
 * 否则 SVN 仓库没有 master，lane 0 会被随便哪条分支借走，SVN 主线断成两列。
 *
 * # trailer 的解析不在这里，在 Rust 侧
 *
 * `git-svn-id:` 与 `Local-Only:` 都写在 commit message 的**正文**里，而
 * `load_graph` 给前端的 `message` 是 `commit.summary()` —— 只有第一段。
 * 2026-09-16 独立 review 发现：在前端逐行找 trailer **恒为 null**，接合、
 * `svn/base` 的 ref 重映射、「仅本地」角标三样一起静默失效，而前端测试因为直接
 * 手写带 trailer 的完整 message 全部照绿（`AGENTS.md` §13 第 9 条的「虚假信心」）。
 *
 * 现在由 `engine/graph.rs` 用 `svn_baseline::parse_trailer` 解析，送到这里时已经是
 * `baselineRev` / `localOnly` 两个字段。那是录制基线时用的**同一份**实现，
 * 两侧规则不会漂移，而之前这里是它的第二份拷贝。
 * 契约由 `src-tauri/tests/svn_graph_contract.rs` 钉住。
 */
import type { CommitNode } from './types'

/** 缓存里的一条 SVN revision，字段与后端 `CachedRev` 逐一对应。 */
export interface SvnRevNode {
  rev: number
  author: string
  /** ISO8601 原样，由本模块负责换成时间戳。 */
  date: string
  message: string
}

/** svn 节点的 id 形式，spec §10.4。 */
function svnId(rev: number): string {
  return `svn:${rev}`
}

/**
 * 接合两条历史（spec §10.3）。
 *
 * ```text
 * ① SVN 序列 → 节点：id = `svn:<rev>`，parents = [下一个更小 rev]
 * ② 扫 git commit，凡带 `baselineRev`（后端解析出的 git-svn-id）的即 baseline，
 *    建 <git id> → svn:<rev> 映射
 * ③ 重写所有 git commit 的 parents，按映射替换；baseline commit 自身从输出里删掉
 * ④ 剔除任何不在最终节点集合里的 parent id        ← spec 没写，见下
 * ```
 *
 * # 第 ④ 步为什么必须有
 *
 * spec §10.3 只写了三步。但**孤儿 baseline 是常态**（`svn update` 会把白名单的
 * revision 推到 HEAD，而那个 revision 往往没碰过白名单），锚点要靠一次额外的
 * 网络请求补拉——断网时就补不到，于是映射表里那个 `svn:N` 在序列里根本不存在。
 *
 * 留一个悬空 parent 不会报错：`computePaths` 里有 `if (!parentNode) continue`，
 * 线画不出来而已。但 `computeOwners` **仍然拿它算归属**，会静默改变车道分配。
 * 所以这里自己剔干净。
 *
 * @param svnRevs 按 revision **降序**（后端 `merged_sequence` 已保证）
 * @param gitCommits 后端 `load_graph` 的产物，顺序是 `--date-order`
 */
/**
 * `<baseline commit 的 oid>` → `svn:<rev>` 的映射。
 *
 * [`spliceHistory`] 内部用它重写 parents；**调用方还要用它重映射两样东西**：
 *
 * - `refs`（`load_graph` 给的分支指针）。baseline commit 被从节点列表里删掉了，
 *   而 `GraphView` 按 `refByHash[node.commit.id]` 渲染标签——不重映射的话
 *   `svn/base` 匹配不到任何一行，**整个看不见**。
 * - `headHash`。用户可以 checkout 到 `svn/base`（Task 13 明确放行），
 *   那时 `headHash` 是 baseline 的 oid，在 commits 里找不到对应行：
 *   HEAD 标记不画、侧边栏单击「定位」滚不过去。
 */
export function baselineMap(gitCommits: CommitNode[]): Map<string, string> {
  const map = new Map<string, string>()
  for (const c of gitCommits) {
    if (c.baselineRev !== undefined) map.set(c.id, svnId(c.baselineRev))
  }
  return map
}

/** 形状与 `engine/types` 的 `RefPointer` 一致，这里只关心 `hash`。 */
interface HasHash {
  hash: string
}

/**
 * 把指向 baseline commit 的 ref 改指对应的 svn 节点。
 *
 * 映射不到的原样返回（普通分支、以及 baseline 但锚点还没拉到的情况）。
 * 幂等：已经是 `svn:N` 的 hash 不在映射表的键里。
 */
export function remapRefs<T extends HasHash>(refs: T[], map: Map<string, string>): T[] {
  return refs.map((r) => {
    const mapped = map.get(r.hash)
    return mapped ? { ...r, hash: mapped } : r
  })
}

export function spliceHistory(
  svnRevs: SvnRevNode[],
  gitCommits: CommitNode[],
  tips: Iterable<string> = [],
): CommitNode[] {
  // ② baseline commit → svn 节点 的映射（① 要用它给 svn 节点挂本地基线，提前算）
  const baselineToSvn = baselineMap(gitCommits)
  // 同一个 revision 可能有多条基线（重建过）：gitCommits 是新的在前，取第一条
  const revToBaseline = new Map<number, string>()
  for (const c of gitCommits) {
    if (c.baselineRev !== undefined && !revToBaseline.has(c.baselineRev)) {
      revToBaseline.set(c.baselineRev, c.id)
    }
  }
  const loaded = new Set(svnRevs.map((r) => svnId(r.rev)))
  // 收起的提交归到哪个 svn 节点：交付录的基线记了真正提交出去的 revision 就认它，
  // 否则（旧基线）退回基线自己的 revision
  const deliveredTo = new Map(baselineToSvn)
  for (const c of gitCommits) {
    if (c.deliveredRev !== undefined && baselineToSvn.has(c.id)) {
      deliveredTo.set(c.id, svnId(c.deliveredRev))
    }
  }
  const delivered = deliveredCommits(gitCommits, deliveredTo, loaded, tips)

  // ① SVN 序列 → 链
  const svnNodes: CommitNode[] = svnRevs.map((r, i) => {
    const node: CommitNode = {
      id: svnId(r.rev),
      // 降序，所以「下一个更小的 revision」就是数组里的下一个
      parents: i + 1 < svnRevs.length ? [svnId(svnRevs[i + 1].rev)] : [],
      author: r.author,
      time: toEpochSeconds(r.date),
      message: r.message,
      vcs: 'svn',
      svnRev: r.rev,
    }
    const own = revToBaseline.get(r.rev)
    if (own !== undefined) node.baselineOid = own
    // 只认图上紧挨着的下一个节点的基线：它没有基线（同事的 revision）时，拿更早的基线
    // 去比会把中间几个 revision 的改动都算到这一个头上
    const prev = i + 1 < svnRevs.length ? revToBaseline.get(svnRevs[i + 1].rev) : undefined
    if (own !== undefined && prev !== undefined) node.diffBase = prev
    const from = delivered.bySvn.get(node.id)
    if (from) node.deliveredFrom = from
    return node
  })

  // ③ 重写 parents，并把 baseline commit 与已交付的提交从输出里去掉
  const rewritten: CommitNode[] = gitCommits
    .filter((c) => !baselineToSvn.has(c.id) && !delivered.hidden.has(c.id))
    .map((c) => ({
      ...c,
      parents: c.parents.map((p) => baselineToSvn.get(p) ?? p),
      vcs: 'git' as const,
      localOnly: c.localOnly ?? false,
    }))

  const all = [...rewritten, ...svnNodes]

  // ④ 剔除悬空 parent
  const known = new Set(all.map((c) => c.id))
  return dateOrder(all.map((c) => ({ ...c, parents: c.parents.filter((p) => known.has(p)) })))
}

/**
 * 把 git、svn 两条各自有序的序列合成一条，规则同 `git log --date-order`：
 * 子一定在父之上；在此前提下越新越靠上，时间相同按输入顺序。
 *
 * 不能直接 `[...git, ...svn]`：分支指着的已交付提交比后面几个 revision 都旧，
 * 却会排到所有 svn 行上面，看起来像是最近才提交的（2026-10-11 真实窗口）。
 * 也不能单纯按时间排：本地或服务器时钟一偏，子就落到父下面，线往上画。
 *
 * 就绪集合的大小是图在某一行的「宽度」（并行的线数），线性扫最大值足够。
 */
function dateOrder(nodes: CommitNode[]): CommitNode[] {
  const index = new Map<string, number>()
  nodes.forEach((c, i) => index.set(c.id, i))
  /** 还没输出的子节点数；归零才轮得到这个节点。 */
  const pending = new Array<number>(nodes.length).fill(0)
  for (const c of nodes) for (const p of c.parents) pending[index.get(p)!]++

  const ready: number[] = []
  nodes.forEach((_, i) => { if (pending[i] === 0) ready.push(i) })
  const newer = (a: number, b: number) =>
    nodes[a].time !== nodes[b].time ? nodes[a].time > nodes[b].time : a < b

  const out: CommitNode[] = []
  while (ready.length > 0) {
    let best = 0
    for (let k = 1; k < ready.length; k++) if (newer(ready[k], ready[best])) best = k
    const i = ready[best]
    ready.splice(best, 1)
    out.push(nodes[i])
    for (const p of nodes[i].parents) {
      const j = index.get(p)!
      if (--pending[j] === 0) ready.push(j)
    }
  }
  return out
}

/**
 * 已交付的本地提交：**只能经由基线 commit 才能到达**的 git 提交（Phase 3a）。
 *
 * 交付录的新基线以交付树为父（spec §8.2.2），交付过的提交于是夹在两条基线之间。
 * 接合把基线换成 svn 节点、svn 节点只连上一个 revision，这些提交就没了子节点：
 * 被画成从旧 revision 分出、悬在那里的分支，还排在所有 svn 行上面，看上去像
 * 「还没交付」（2026-10-11 真实窗口截图）。它们的内容已经是 SVN 历史的一部分，
 * 所以不进图，记到对应 svn 节点的 `deliveredFrom` 上。
 *
 * 判据：有子节点、子节点全是基线或已隐藏的提交、且没有任何 ref / HEAD 直接指着。
 * 「还有分支指着」或「另一条分支从它上面长出来」的照常显示——不能把用户的分支藏掉。
 * `load_graph` 的提交全部从 ref / HEAD 走出来，所以没有子节点的提交一定被指着。
 *
 * `gitCommits` 是拓扑序（子在父之前），一遍扫完即可：轮到某个提交时它的子节点都已判过。
 */
function deliveredCommits(
  gitCommits: CommitNode[],
  /** 基线 commit → 收在它下面的提交归哪个 svn 节点（键集就是全部基线）。 */
  deliveredTo: Map<string, string>,
  loaded: Set<string>,
  tips: Iterable<string>,
): { hidden: Set<string>; bySvn: Map<string, CommitNode[]> } {
  const pinned = new Set(tips)
  // 追加用 push 而不是展开复制：一次交付一长串提交时，展开是每个键 O(k²)（最终 review 第 9 条）
  const append = <T>(m: Map<string, T[]>, k: string, v: T) => {
    const list = m.get(k)
    if (list) list.push(v)
    else m.set(k, [v])
  }
  const children = new Map<string, string[]>()
  for (const c of gitCommits) {
    for (const p of c.parents) append(children, p, c.id)
  }
  // 基线对应的 revision 还没拉进来（首次打开、只翻了一页）时不算数：收进一个不画的节点，
  // 交付的提交就整个看不见了（最终 review 第 6 条）。拉到之后下一次接合自然收起
  const settled = (k: string) => {
    const svn = deliveredTo.get(k)
    return svn !== undefined && loaded.has(svn)
  }
  const hidden = new Set<string>()
  // 隐藏的提交记到哪个 svn 节点：沿子节点往上找到的第一条基线
  const owner = new Map<string, string>()
  for (const c of gitCommits) {
    if (deliveredTo.has(c.id) || pinned.has(c.id)) continue
    const kids = children.get(c.id) ?? []
    if (kids.length === 0) continue
    if (!kids.every((k) => settled(k) || hidden.has(k))) continue
    hidden.add(c.id)
    const k = kids[0]
    const svn = deliveredTo.get(k) ?? owner.get(k)
    if (svn !== undefined) owner.set(c.id, svn)
  }
  const bySvn = new Map<string, CommitNode[]>()
  for (const c of gitCommits) {
    const svn = owner.get(c.id)
    if (svn === undefined) continue
    append(bySvn, svn, c)
  }
  return { hidden, bySvn }
}

/**
 * 当前选中该读哪段 diff。
 *
 * 图上的节点交给 [`commitDiffScope`]；**不在图上的选中照旧按 oid 读**——stash 提交不被任何
 * 分支引用、不在 `load_graph` 里，5000 条上限之外的提交也一样，`RepoTab` 的 `detailCommit`
 * 专门为它们拼了节点。只认图上节点的话，点 stash 不再读 diff，详情停在上一条提交上，
 * 而且普通 git 仓库也中招（最终 review 第 1 条）。不在图上的 `svn:N` 不是 oid，不请求。
 */
export function selectionDiffScope(commits: CommitNode[], selectedId: string | null): string | null {
  if (!selectedId) return null
  const node = commits.find((c) => c.id === selectedId)
  if (node) return commitDiffScope(node)
  return selectedId.startsWith('svn:') ? null : `commit:${selectedId}`
}

/**
 * svn 节点为什么本地算不出文件变化：`'self'` 自己没在本机记过（同事刚提交、还没更新），
 * `'prev'` 自己有、图上前一个没有（同事的 revision 之后交付或更新的）。能算或不是 svn 节点时 `null`。
 *
 * 两种说法不能混：同事的 r18 紧跟在我交付的 r17 之后时，说「前一个 revision 没在本机记过」
 * 是错的（2026-10-15 演示）。
 */
export function svnDiffGap(c: CommitNode): 'self' | 'prev' | null {
  if (c.vcs !== 'svn' || (c.baselineOid && c.diffBase)) return null
  return c.baselineOid ? 'prev' : 'self'
}

/**
 * svn 节点详情里要不要列出它收起来的本地提交，列哪些。
 *
 * 只有一个提交、标题又和 SVN 信息一样、下面也能看到它的改动时，这一栏只是把上面再说一遍
 * （2026-10-15 用户反馈），不显示。其余情况照常列出：多个提交、交付时改写了信息，或者这里
 * 算不出文件变化——这一栏是点开它们看改动的唯一入口（2026-10-16 review）。
 */
export function deliveredNote(c: CommitNode): CommitNode[] | null {
  const from = c.deliveredFrom
  if (c.vcs !== 'svn' || !from || from.length === 0) return null
  const redundant =
    from.length === 1 &&
    c.baselineOid !== undefined &&
    c.diffBase !== undefined &&
    from[0].message.trim() === summaryOf(c.message)
  return redundant ? null : from
}

/**
 * 与 git 的 `summary()` 同样取法：第一段（到第一个空行为止），段内换行并成空格、空白压成一个。
 * git 侧的 `message` 就是这样来的，SVN 信息只取第一行的话，跨行的标题永远对不上（2026-10-16 review）。
 */
function summaryOf(message: string): string {
  const firstParagraph = message.trim().split(/\r?\n\s*\r?\n/)[0]
  return firstParagraph.replace(/\s+/g, ' ').trim()
}

/**
 * 选中的是不是收进 svn 节点、不在图上的本地提交；是的话连同收着它的 svn 节点一起返回。
 *
 * 详情据此显示这个提交本身、并给「返回 rN」。它的 diff 不用另接：不在图上的选中本来就按
 * oid 读（[`selectionDiffScope`]）。
 */
export function findDelivered(
  commits: CommitNode[],
  id: string | null,
): { commit: CommitNode; owner: CommitNode } | null {
  if (!id) return null
  for (const owner of commits) {
    const commit = owner.deliveredFrom?.find((d) => d.id === id)
    if (commit) return { commit, owner }
  }
  return null
}

/**
 * 选中的节点该读哪段 diff（`read_diff` 的 scope）。
 *
 * svn 节点的 id 是 `svn:N`，不是 oid，原样发 `commit:svn:N` 只会在后端解析失败。
 * 它前后两版都有本地基线时，两版基线之差就是这一步在白名单里改了什么，本地就能算，
 * 不必等 Phase 4 的 `svn diff -c`。
 */
export function commitDiffScope(c: CommitNode | undefined): string | null {
  if (!c) return null
  if (c.vcs !== 'svn') return `commit:${c.id}`
  return c.baselineOid && c.diffBase ? `range:${c.diffBase}..${c.baselineOid}` : null
}

/**
 * ISO8601 → 秒级时间戳，与 `CommitNode.time` 的单位一致。
 *
 * 解析不了时返回 0 而不是 `NaN`：`NaN` 传进 `new Date(NaN * 1000)` 会渲染成
 * `Invalid Date`，而 0 至少是一个能显示的时间。日期本身是展示用的，
 * 图的拓扑一点都不依赖它。
 */
function toEpochSeconds(iso: string): number {
  const ms = Date.parse(iso)
  return Number.isNaN(ms) ? 0 : Math.floor(ms / 1000)
}
