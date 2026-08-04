export interface RepoInfo {
  path: string
  head: string
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
