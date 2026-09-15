/**
 * 混合图：把 SVN 历史与本地 git 历史接成一张图（spec §10.3）。
 *
 * 这一层是**纯函数**：无 React、无 Tauri 依赖，可直接单测（`AGENTS.md` §7.2）。
 * `engine/layout.ts` 与 `engine/svgRenderer.tsx` **一行都不用改**——它们只认
 * `parents`（spec §10.4）。
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

/**
 * `git-svn-id` trailer 的格式，spec §6.4.1 钉死：
 * `git-svn-id: <url>@<rev> <repository-uuid>`，**三个字段缺一不可**。
 *
 * ⚠️ Rust 侧 `svn_baseline::parse_trailer` 有同一条正则的另一份实现。
 * 格式由 spec 钉死且两侧各有测试，接受这处重复——把它做成一个跨语言的单一事实源
 * 需要代码生成，代价大于收益。
 */
const GIT_SVN_ID = /^git-svn-id: (\S+)@(\d+) ([0-9a-fA-F-]+)$/

/** `Local-Only: true`（spec §6.7）。键按 git 的惯例大小写不敏感。 */
const LOCAL_ONLY = /^local-only:\s*true\s*$/i

export interface GitSvnId {
  url: string
  rev: number
  uuid: string
}

/**
 * 从 commit message 里解析 `git-svn-id` trailer。
 *
 * **逐行完整匹配，不用 `includes`。** 正文里顺口提到这个格式的 commit 会被
 * 误判成 baseline，而 baseline 在 [`spliceHistory`] 里是要**从图上删掉**的
 * ——一个普通提交会就此消失。
 *
 * 解析失败即视为该 commit 不是 baseline：不猜、不部分接受（spec §6.4.1）。
 */
export function parseGitSvnId(message: string): GitSvnId | null {
  for (const line of message.split('\n')) {
    const m = GIT_SVN_ID.exec(line.trim())
    if (m) return { url: m[1], rev: Number(m[2]), uuid: m[3] }
  }
  return null
}

/**
 * 这个 commit 是不是「仅本地，不交付」（spec §6.7）。
 *
 * 判定**只认这条 trailer**，不新增任何配置。同样逐行完整匹配。
 */
export function isLocalOnly(message: string): boolean {
  return message.split('\n').some((line) => LOCAL_ONLY.test(line.trim()))
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
 * ② 扫 git commit，凡能解析出 git-svn-id 的即 baseline，建 <git id> → svn:<rev> 映射
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
  const baselineToSvn = new Map<string, string>()
  for (const c of gitCommits) {
    const trailer = parseGitSvnId(c.message)
    if (trailer) baselineToSvn.set(c.id, svnId(trailer.rev))
  }

  // ③ 重写 parents，并把 baseline commit 自身从输出里去掉
  const rewritten: CommitNode[] = gitCommits
    .filter((c) => !baselineToSvn.has(c.id))
    .map((c) => ({
      ...c,
      parents: c.parents.map((p) => baselineToSvn.get(p) ?? p),
      vcs: 'git' as const,
      localOnly: isLocalOnly(c.message),
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
