export interface Worktree {
  name: string
  path: string
  head: string
  branch: string
  isMain: boolean
}

export interface RepoInfo {
  path: string
  /** 给人看的分支名。 */
  head: string
  /** HEAD 的完整 oid，工作区伪行挂在它上面。 */
  headHash: string
}

export interface RefPointer {
  name: string
  hash: string
  kind: 'head' | 'branch' | 'remote' | 'tag' | 'other'
  /**
   * 本地分支相对其上游的领先/落后数；没有上游或不是本地分支时为 null。
   *
   * null 和 0 含义不同：没上游时不画徽章，同步时画个 0 也是噪音。
   */
  ahead: number | null
  behind: number | null
  /**
   * 配置了上游、但上游 ref 已不存在时，上游的完整 ref 名（如
   * `refs/remotes/origin/feature`）；其余情况为 null。
   *
   * 即 `git branch -vv` 的 `[origin/feature: gone]`。此时 graph 上没有远程标签、
   * 也没有 ↑↓，不看这个字段就和「从没推过」分不开。
   */
  goneUpstream: string | null
}

export interface CommitNode {
  /** git 节点是 oid；svn 节点是 `svn:12346`（spec §10.4）。 */
  id: string
  parents: string[]
  author: string
  time: number
  /**
   * **只有标题。** 后端取的是 `commit.summary()`（第一段，空白已折叠），
   * 正文不在里面。
   *
   * ⚠️ 不要在这里找 trailer。2026-09-16 独立 review 之前，混合图就是从这个字段
   * 逐行找 `git-svn-id:` 的 —— 那一行永远不可能出现在这里，于是接合、
   * `svn/base` 的 ref 重映射、「仅本地」角标**三样全部静默失效**，
   * 而两侧测试全绿（前端测试直接手写带 trailer 的 message，跳过了断掉的那一环）。
   * trailer 现在由 Rust 侧 `parse_trailer` 解析，结论见下面两个字段。
   */
  message: string
  /**
   * 新增（spec §10.4）。**缺省视为 git**，所以现有 Git 模式零影响。
   */
  vcs?: 'git' | 'svn'
  /** 新增；仅 svn 节点有，值是这个节点自己的 revision。 */
  svnRev?: number
  /**
   * 后端解析出的 `git-svn-id` trailer 里记的 revision（spec §6.4.1）：
   * 这个 git commit **是一条基线**，记的是那个 SVN revision。
   *
   * 与 `svnRev` 不是一回事：`svnRev` 说「我就是这条 revision」，
   * 这个说「我是记录那条 revision 的基线 commit」。基线在接合时会被从图上删掉。
   */
  baselineRev?: number
  /**
   * 交付录的基线额外记的「真正提交出去的 revision」（`Delivered-Rev:`）。`baselineRev` 是交付之后
   * 更新到的 revision，同事紧跟着提交时它是同事的；收起的本地提交按这个归属。其余基线缺省。
   */
  deliveredRev?: number
  /** 后端解析出的 `Local-Only: true`（spec §6.7），不会被交付。 */
  localOnly?: boolean
  /**
   * 仅 svn 节点：这个 revision 交付出去、因此不再单独画在图上的本地提交
   * （`spliceHistory` 的「已交付」规则）。没有时缺省。
   *
   * 按基线的 `deliveredRev` 归属；早于 2026-10-16 录的基线没有它，退回按 `baselineRev`——
   * 那时交付之后同事若紧跟着提交过，会挂在同事的 revision 上。
   *
   * 存整条提交（`load_graph` 给的原样，parents 未改写）：详情里点开单个提交要显示作者、时间、父提交。
   */
  deliveredFrom?: CommitNode[]
  /** 仅 svn 节点：记录这个 revision 的本地基线 commit（有多条时取最新）。 */
  baselineOid?: string
  /** 仅 svn 节点：图上下一个（更早的）svn 节点的本地基线。两者都在才能本地算文件变化。 */
  diffBase?: string
}

export interface GraphData {
  commits: CommitNode[]
  refs: RefPointer[]
}

/**
 * 文件相对上一版本的状态。`FileChange`（工作区）和 `FileDiff`（任意 diff）
 * 共用同一套取值，文件树的状态角标只认这一套。
 */
export type FileStatus = 'modified' | 'added' | 'deleted' | 'renamed' | 'copied' | 'conflicted'

export interface FileChange {
  path: string
  status: FileStatus
  additions: number
  deletions: number
}

export interface StatusData {
  staged: FileChange[]
  unstaged: FileChange[]
}

export interface DiffLine {
  origin: '+' | '-' | ' '
  content: string
  /** 旧版本里的行号；新增行为 null。 */
  oldLineno: number | null
  /** 新版本里的行号；删除行为 null。 */
  newLineno: number | null
}

export interface DiffHunk {
  oldStart: number
  newStart: number
  /** git 原样的 `@@ -a,b +c,d @@ 上下文` 头。 */
  header: string
  lines: DiffLine[]
}

export interface FileDiff {
  path: string
  status: FileStatus
  /** 重命名前的路径；其余情况为 null。 */
  oldPath: string | null
  /** 二进制文件没有可显示的行。 */
  binary: boolean
  hunks: DiffHunk[]
}

export interface DiffData {
  files: FileDiff[]
}

export interface UpstreamInfo {
  /** 当前分支名，不带 refs/heads/ 前缀。 */
  branch: string
  /** 上游简称如 "origin/master"；没设上游时为 null。 */
  upstream: string | null
  ahead: number
  behind: number
}

export interface StashEntry {
  /** stash@{N} 里的 N，0 最新。 */
  index: number
  message: string
  /** stash 本身是个提交，可直接用于 `commit:<hash>` diff scope。 */
  hash: string
  author: string
  /** 提交时间，Unix 秒。 */
  time: number
  /** 父提交。stash 是个 merge 提交，第一父是 stash 时所在的那个提交。 */
  parents: string[]
}

/**
 * 一条 open 状态的 GitHub Pull Request，字段对齐 `gh pr list --json` 的输出。
 *
 * 它不是 git 的概念，所以刻意不挂进 [`RefPointer`]：PR 走的是另一条会失败、
 * 会超时的网络通道，塞进 graph 数据结构会让本地读图也背上网络状态。
 */
export interface PullRequest {
  number: number
  /** PR 的来源分支名，不带 refs/heads/ 前缀，如 `feat/x`。 */
  headRefName: string
  title: string
  isDraft: boolean
  /**
   * gh 原样返回的状态。
   *
   * `CLOSED` 指**关闭但没合并**，与 `MERGED` 是两回事——git 客户端里把这两者
   * 混为一谈会给出完全相反的暗示（「可以删分支了」vs「这活白干了」）。
   */
  state: 'OPEN' | 'MERGED' | 'CLOSED'
  /**
   * PR 的 head 分支在**合并时**的提交 oid。
   *
   * 和本地分支的 tip 一比，就知道 PR 合并之后分支有没有继续往前走。没有这个
   * 判断的话，一条「合并后又加了提交」的分支会被画成普通的 ✓——照着它删分支
   * 会把那些提交一起删掉。
   */
  headRefOid: string
  url: string
  /**
   * 本地同名分支相对合并点的位置，gh 不给，由后端在本地补算。
   *
   * 只有已合并、有同名本地分支、且合并点对象在本地的 PR 才有值；其余为 null。
   * 可选是为了兼容没有这个字段的老后端，缺失与 null 同义。
   */
  localRelation?: LocalRelation | null
}

/** 本地分支相对 PR 合并点的领先/落后数，即 `git rev-list --left-right --count <tip>...<合并点>`。 */
export interface LocalRelation {
  /** 计算时本地分支的 tip。分支之后动过的话，这份结论就作废了。 */
  tip: string
  /** 本地分支有、合并点没有的提交数。 */
  ahead: number
  /** 合并点有、本地分支没有的提交数。 */
  behind: number
}

/** 冲突文件某个 stage 的条目，与 Rust 侧 `StageInfo` 一一对应。 */
export interface StageInfo {
  oid: string
  /** git 的文件 mode，例如 0o100644；0o120000 是符号链接。 */
  mode: number
}

/** 冲突的五种情况（spec §4.1），判定顺序 ⑤ → ③ → ④ → ① → ②。 */
export type ConflictCase = 'structural' | 'deleteModify' | 'notTextMerged' | 'markers' | 'noMarkers'

/** ④ git 没做逐行文本合并的原因。 */
export type NotTextReason = 'binaryAttr' | 'customDriver' | 'mergeDefault' | 'lfs' | 'binaryContent'

/** 一个冲突文件的信息，与 Rust 侧 `ConflictInfo` 一一对应（`read_conflicts`）。 */
export interface ConflictInfo {
  path: string
  /** `UU` / `AA` / `DU` / `UD` / `AU` / `UA` / `DD`。 */
  code: string
  base: StageInfo | null
  ours: StageInfo | null
  theirs: StageInfo | null
  case: ConflictCase
  reason: NotTextReason | null
  /** 自定义 merge driver 的名字。 */
  driver: string | null
  /** 工作区（经过过滤器）与 HEAD 完全相同。 */
  sameAsHead: boolean
}
