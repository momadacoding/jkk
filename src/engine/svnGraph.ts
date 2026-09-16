/**
 * 混合图：把 SVN 历史与本地 git 历史接成一张图（spec §10.3）。
 *
 * 这一层是**纯函数**：无 React、无 Tauri 依赖，可直接单测（`AGENTS.md` §7.2）。
 * `engine/layout.ts` 与 `engine/svgRenderer.tsx` **一行都不用改**——它们只认
 * `parents`（spec §10.4）。
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

export function spliceHistory(svnRevs: SvnRevNode[], gitCommits: CommitNode[]): CommitNode[] {
  // ① SVN 序列 → 链
  const svnNodes: CommitNode[] = svnRevs.map((r, i) => ({
    id: svnId(r.rev),
    // 降序，所以「下一个更小的 revision」就是数组里的下一个
    parents: i + 1 < svnRevs.length ? [svnId(svnRevs[i + 1].rev)] : [],
    author: r.author,
    time: toEpochSeconds(r.date),
    message: r.message,
    vcs: 'svn',
    svnRev: r.rev,
  }))

  // ② baseline commit → svn 节点 的映射
  const baselineToSvn = baselineMap(gitCommits)

  // ③ 重写 parents，并把 baseline commit 自身从输出里去掉
  const rewritten: CommitNode[] = gitCommits
    .filter((c) => !baselineToSvn.has(c.id))
    .map((c) => ({
      ...c,
      parents: c.parents.map((p) => baselineToSvn.get(p) ?? p),
      vcs: 'git' as const,
      localOnly: c.localOnly ?? false,
    }))

  const all = [...rewritten, ...svnNodes]

  // ④ 剔除悬空 parent
  const known = new Set(all.map((c) => c.id))
  return all.map((c) => ({ ...c, parents: c.parents.filter((p) => known.has(p)) }))
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
