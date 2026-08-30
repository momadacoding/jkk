import type { PullRequest, RefPointer } from './types'

/** 本地分支的全限定前缀。只在 [`pullRequestForRef`] 里剥，且只剥一次。 */
const LOCAL_BRANCH_PREFIX = 'refs/heads/'

/**
 * 把 PR 列表按 head 分支名建成索引，供分支行 O(1) 查徽章。
 *
 * 匹配是**精确的**，不做大小写折叠：GitHub 的分支名大小写敏感，折叠会把
 * `Feat/X` 的 PR 错挂到本地的 `feat/x` 上。徽章指错 PR 比不显示徽章更糟。
 *
 * 同一个 head 分支可以同时开多个 open PR（指向不同 base），而徽章只有一个位置，
 * 取编号最大的那条——最贴近「我刚开的那个 PR」，也不依赖 gh 的返回顺序。
 */
export function indexPullRequestsByBranch(prs: PullRequest[]): Map<string, PullRequest> {
  const index = new Map<string, PullRequest>()
  for (const pr of prs) {
    // 空 key 会被「分支名为空」的查询意外命中，而那本身就是坏数据
    const branch = pr.headRefName?.trim()
    if (!branch) continue
    const seen = index.get(branch)
    if (!seen || pr.number > seen.number) index.set(branch, pr)
  }
  return index
}

/**
 * 这条 ref 该不该显示 PR 编号。
 *
 * **只有本地分支才给编号。** 远程分支被排除不是因为匹配不上——`origin/feat/x`
 * 剥掉前缀后正好也是 PR 的 head 分支名，反而会命中——而是因为同一个提交上
 * 经常同时挂着 `feat/x` 和 `origin/feat/x` 两个标签，两边都标就成了重复噪音。
 * tag 和 HEAD 伪 ref 同理，它们根本不是 PR 的来源。
 */
export function pullRequestForRef(
  index: Map<string, PullRequest>,
  ref: Pick<RefPointer, 'name' | 'kind'>,
): PullRequest | undefined {
  if (ref.kind !== 'branch') return undefined
  const branch = ref.name.startsWith(LOCAL_BRANCH_PREFIX)
    ? ref.name.slice(LOCAL_BRANCH_PREFIX.length)
    : ref.name
  return index.get(branch)
}
