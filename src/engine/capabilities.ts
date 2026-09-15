/**
 * 后端能力声明。字段与 Rust 侧 `vcs::Capabilities` 一一对应。
 *
 * Tauri 把 Rust 的 snake_case 转成 JS 的 camelCase（`pull_requests` →
 * `pullRequests`），这层转换**没有编译期检查**，写错只会在运行时静默拿到
 * `undefined`——而 `undefined` 是 falsy，表现就是「入口莫名其妙消失了」。
 * 所以这个接口的字段名必须与 Rust 侧逐字对齐（AGENTS.md §7.3）。
 *
 * 这一层是**表现层**防线，不是接缝：`checkout_remote_branch` / `create_tag` /
 * `cherry_pick` / `save_patch` / `fetch` / `pull` / `push` 与整个
 * `commands/worktree.rs` 都还直连 git CLI，没有经过 `trait Vcs`。隐藏入口只是
 * 不让人点到它们，不是它们变安全了。
 */
export interface Capabilities {
  /**
   * 这个仓库背后是哪种版本控制。
   *
   * 前端据此决定要不要发 SVN 专属的 invoke。**不要**用「所有 git 专属能力都是
   * false」去反推——那在今天成立纯属巧合，将来某个普通 git 仓库恰好关掉全部
   * 可选能力（或者新增一个默认关闭的能力）时，它就会被当成 SVN 仓库，
   * 于是每次刷新都发几个必然失败的 invoke。
   */
  vcs: 'git' | 'svn'
  worktrees: boolean
  tags: boolean
  remotes: boolean
  pullRequests: boolean
  cherryPick: boolean
  patches: boolean
}

/** 普通 git 仓库：全支持。也是拿不到后端应答时的保守默认值。 */
export const ALL_CAPABILITIES: Capabilities = {
  vcs: 'git',
  worktrees: true,
  tags: true,
  remotes: true,
  pullRequests: true,
  cherryPick: true,
  patches: true,
}

/**
 * 全部 git 专属功能都关掉，即 SVN 混合仓库当前声明的形态。
 *
 * 存在的意义是让「加了新字段却忘了在 SVN 侧关掉」这件事有测试能抓到
 * ——只写 `ALL_CAPABILITIES` 的话，新字段默认就是开着的。
 */
export const NO_GIT_ONLY_CAPABILITIES: Capabilities = {
  vcs: 'svn',
  worktrees: false,
  tags: false,
  remotes: false,
  pullRequests: false,
  cherryPick: false,
  patches: false,
}

/** 侧边栏分区。取值与 `RepoTab` 里的渲染顺序一致。 */
export type SidebarSection = 'branches' | 'tags' | 'worktrees' | 'stashes'

/**
 * 侧边栏该渲染哪些分区，**按渲染顺序**返回。
 *
 * `branches` 与 `stashes` **没有开关**：§16 Phase 1 的验收标准是「本地 git 的
 * 暂存/分支/stash 可用」，SVN 模式下它们照常工作。
 *
 * 远程分支与 PR 不在这个列表里——它们是 `BRANCHES` 分区**内部**的分组，
 * 由 `BranchView` 自己按 `caps.remotes` / `caps.pullRequests` 决定。
 */
export function visibleSections(caps: Capabilities): SidebarSection[] {
  const out: SidebarSection[] = ['branches']
  if (caps.tags) out.push('tags')
  if (caps.worktrees) out.push('worktrees')
  out.push('stashes')
  return out
}
