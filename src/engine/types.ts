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

export interface FileChange {
  path: string
  status: 'modified' | 'added' | 'deleted' | 'renamed' | 'conflicted'
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
}

export interface DiffHunk {
  oldStart: number
  newStart: number
  lines: DiffLine[]
}

export interface FileDiff {
  path: string
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
