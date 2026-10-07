import type { PullRequest, RefPointer } from './types'

/** 本地分支的全限定前缀。只在 [`pullRequestForRef`] 里剥，且只剥一次。 */
const LOCAL_BRANCH_PREFIX = 'refs/heads/'

/**
 * 徽章的形态。CLOSED（关闭未合并）不在其中，它根本不显示。
 *
 * 已合并的 PR 按本地分支相对合并点的位置再分五种：
 * - `merged`：分支头就在合并点上，内容全在主干。
 * - `merged-ahead`：合并后分支上又长了提交。**危险**——画成普通的 ✓ 等于告诉用户
 *   「可以删了」，而那会丢东西。
 * - `merged-behind`：合并点比本地新（别处推了提交、本地没拉）。本地没有 PR 之外的
 *   提交，和 `merged` 一样安全，但分支头对不上，要说清楚为什么。
 * - `merged-diverged`：两边各有对方没有的提交。危险，同 ahead。
 * - `merged-unknown`：分支头对不上，但判断不了方向（合并点对象本地没有，或分支在
 *   上次读取 PR 之后又动过）。可能是 ahead，所以按危险处理。
 */
export type PrBadgeVariant =
  | 'open'
  | 'draft'
  | 'merged'
  | 'merged-ahead'
  | 'merged-behind'
  | 'merged-diverged'
  | 'merged-unknown'

/** 徽章渲染需要的全部东西。组件只调 [`prBadge`] 一次，不各自拼字符串。 */
export interface PrBadgeView {
  variant: PrBadgeVariant
  /** 徽章上的文字，如 `#37`、`✓#33`、`✓#20↑`。 */
  label: string
  /** 悬停提示。 */
  tooltip: string
}

/** 取前 7 位，和界面上别处显示 hash 的长度一致。 */
const shortOid = (oid: string) => oid.slice(0, 7)

function variantOf(pr: PullRequest, branchHash?: string): PrBadgeVariant {
  // MERGED 压过 isDraft：gh 不该给出「已合并的草稿」，但真给了也不能显示成
  // 灰色草稿——那会让一条已经进主干的分支看起来还没开始 review
  //
  // 下面的分支头比对只对 MERGED 做。open PR 的分支头对不上意味着有没推上去的提交，
  // 那件事分支行上的 ahead/behind 徽章已经在说了，不重复报警。
  if (pr.state !== 'MERGED') return pr.isDraft ? 'draft' : 'open'

  // 分支头还停在合并点上吗？`headRefOid` 是 PR **合并时**的分支头，
  // 和本地分支的 tip 一比就知道分支有没有继续往前走。
  //
  // 两个都拿得到才敢下判断：老版本 gh 可能没有这个字段，远程分支标签也没有
  // 对应的本地 tip。宁可少一个提示，也不能凭空报警。
  if (!branchHash || !pr.headRefOid) return 'merged'
  const tip = branchHash.toLowerCase()
  if (tip === pr.headRefOid.toLowerCase()) return 'merged'

  // 对不上时方向由后端的祖先计算给出——光比 SHA 的话，「本地落后」会被说成
  // 「本地领先」。这份结论是在读 PR 列表那一刻对着当时的分支头算的，PR 列表
  // 只在 fetch/pull/push 后刷新，分支却随时会动：头已经变了就不能再用。
  const rel = pr.localRelation
  if (!rel || rel.tip.toLowerCase() !== tip) return 'merged-unknown'
  if (rel.ahead > 0 && rel.behind > 0) return 'merged-diverged'
  if (rel.ahead > 0) return 'merged-ahead'
  if (rel.behind > 0) return 'merged-behind'
  // 0/0 却 SHA 不同只可能是后端数据自相矛盾，不当成「已全部合并」
  return 'merged-unknown'
}

/**
 * 算出一条 PR 在分支上该画成什么样。
 *
 * `branchHash` 是**本地分支的 tip**（`RefPointer.hash`）。它只用来细分已合并的
 * 几种形态，其余形态不需要。
 *
 * 标签里的 `✓` 和 `↑` `↓` `?` **不是装饰**：graph 里徽章的底色是车道色，12 条车道各不
 * 相同；能过对比度的紫已经淡到与 draft 灰分不开（实测最高 4.41）。颜色在这里
 * 当不了唯一的区分手段，符号才能，而且这一路对色盲同样成立。`↑` `↓` 沿用侧边栏
 * ahead/behind 徽章的箭头，在这个界面里已经是「本地领先/落后」的意思。
 */
export function prBadge(pr: PullRequest, branchHash?: string): PrBadgeView {
  const variant = variantOf(pr, branchHash)
  // 合并点与分支头不一致的几种形态，两个 SHA 都要说出来，否则用户没法自己核对
  const title = `已合并的 PR #${pr.number}: ${pr.title}\n`
  const mergedAt = shortOid(pr.headRefOid)
  const local = shortOid(branchHash ?? '')
  const ahead = pr.localRelation?.ahead ?? 0
  const behind = pr.localRelation?.behind ?? 0
  switch (variant) {
    case 'merged':
      return {
        variant,
        label: `✓#${pr.number}`,
        tooltip: `${title}分支内容已全部进入主干。`,
      }
    case 'merged-ahead':
      return {
        variant,
        label: `✓#${pr.number}↑`,
        tooltip:
          `${title}合并的是 ${mergedAt}，本地分支已经走到 ${local}，` +
          `多出 ${ahead} 个提交不在这个 PR 里——删分支前先确认。`,
      }
    case 'merged-behind':
      return {
        variant,
        label: `✓#${pr.number}↓`,
        // 这种情况最常见的来历是「在另一台机器上补了提交再合并」，远端分支随后被删，
        // 图上就再也找不到那几个提交。说清楚它们去了哪，用户才不会以为丢了东西。
        tooltip:
          `${title}合并的是 ${mergedAt}，比本地分支 ${local} 新 ${behind} 个提交` +
          `（在别处推上去、本地没拉）。\n本地分支没有 PR 之外的提交，内容已全部进入主干。`,
      }
    case 'merged-diverged':
      return {
        variant,
        label: `✓#${pr.number}↑↓`,
        tooltip:
          `${title}合并的是 ${mergedAt}，本地分支 ${local} 与它分叉：` +
          `本地多出 ${ahead} 个提交不在这个 PR 里，PR 里另有 ${behind} 个提交本地没有——删分支前先确认。`,
      }
    case 'merged-unknown':
      return {
        variant,
        label: `✓#${pr.number}?`,
        tooltip:
          `${title}合并的是 ${mergedAt}，本地分支在 ${local}，判断不了两者的先后` +
          `（合并点的提交本地没有，或分支在上次读取 PR 之后又动过；fetch 后会重新判断）。\n` +
          `本地可能有不在这个 PR 里的提交——删分支前先确认。`,
      }
    case 'draft':
      return { variant, label: `#${pr.number}`, tooltip: `Draft PR #${pr.number}: ${pr.title}` }
    default:
      return { variant, label: `#${pr.number}`, tooltip: `PR #${pr.number}: ${pr.title}` }
  }
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
