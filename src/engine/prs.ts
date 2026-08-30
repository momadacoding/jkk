import type { PullRequest, RefPointer } from './types'

/** 本地分支的全限定前缀。只在 [`pullRequestForRef`] 里剥，且只剥一次。 */
const LOCAL_BRANCH_PREFIX = 'refs/heads/'

/**
 * 徽章的三种形态。CLOSED（关闭未合并）不在其中，它根本不显示。
 */
export type PrBadgeVariant = 'open' | 'draft' | 'merged'

/**
 * 一条 PR 该画成哪种徽章。
 *
 * `MERGED` 压过 `isDraft`：gh 不该给出「已合并的草稿」，但真给了也不能显示成
 * 灰色草稿——那会让一条已经进主干的分支看起来还没开始 review。
 */
export function prBadgeVariant(pr: PullRequest): PrBadgeVariant {
  if (pr.state === 'MERGED') return 'merged'
  return pr.isDraft ? 'draft' : 'open'
}

/**
 * 徽章上的文字。
 *
 * merged 带一个 `✓` 前缀，**这不是装饰**。graph 里徽章的底色是车道色，12 条
 * 车道各不相同；紫色在最亮的黄色车道上淡到与 draft 灰分不开（实测最高只能到
 * 4.41，再深就过不了对比度）。颜色在这里当不了唯一的区分手段，符号才能，
 * 而且这一路对色盲同样成立。
 */
export function prBadgeLabel(pr: PullRequest): string {
  return prBadgeVariant(pr) === 'merged' ? `✓#${pr.number}` : `#${pr.number}`
}

/** 徽章形态到 tooltip 措辞。 */
const VARIANT_TITLE: Record<PrBadgeVariant, string> = {
  open: 'PR',
  draft: 'Draft PR',
  merged: '已合并的 PR',
}

/**
 * 徽章的悬停提示。
 *
 * 光一个 `#37` 说不清是哪个 PR，标题必须能看到；merged 还要说清状态，
 * 否则用户会以为那是一个还开着的 PR。
 */
export function prTooltip(pr: PullRequest): string {
  return `${VARIANT_TITLE[prBadgeVariant(pr)]} #${pr.number}: ${pr.title}`
}

/** OPEN 优先于 MERGED。数字越大越优先，CLOSED 直接出局。 */
function statePriority(pr: PullRequest): number {
  if (pr.state === 'OPEN') return 2
  if (pr.state === 'MERGED') return 1
  return 0
}

/**
 * 把 PR 列表按 head 分支名建成索引，供分支行 O(1) 查徽章。
 *
 * 匹配是**精确的**，不做大小写折叠：GitHub 的分支名大小写敏感，折叠会把
 * `Feat/X` 的 PR 错挂到本地的 `feat/x` 上。徽章指错 PR 比不显示徽章更糟。
 *
 * 一个分支可能对应多条 PR，而徽章只有一个位置，挑选规则是：
 *
 * 1. **关闭但未合并的直接丢掉。** 被否掉的 PR 对「这条分支现在什么状态」没有
 *    信息量，标出来只会误导。
 * 2. **OPEN 优先于 MERGED，哪怕编号更小。** 老分支合过一次又重开 PR 时，
 *    当下正在进行的那个才是用户关心的。
 * 3. 同状态下取编号最大的，即最新的一条；这样也不依赖 gh 的返回顺序。
 */
export function indexPullRequestsByBranch(prs: PullRequest[]): Map<string, PullRequest> {
  const index = new Map<string, PullRequest>()
  for (const pr of prs) {
    const priority = statePriority(pr)
    if (priority === 0) continue
    // 空 key 会被「分支名为空」的查询意外命中，而那本身就是坏数据
    const branch = pr.headRefName?.trim()
    if (!branch) continue
    const seen = index.get(branch)
    if (!seen) {
      index.set(branch, pr)
      continue
    }
    const seenPriority = statePriority(seen)
    if (priority > seenPriority || (priority === seenPriority && pr.number > seen.number)) {
      index.set(branch, pr)
    }
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
