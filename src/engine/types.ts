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
}

export interface CommitNode {
  id: string
  parents: string[]
  author: string
  time: number
  message: string
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
  url: string
}
