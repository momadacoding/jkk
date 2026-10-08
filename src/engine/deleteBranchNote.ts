import type { PullRequest } from './types'

/**
 * 删除分支确认框里「未合并提交」那一行要用到的字段，`DeleteInfo` 结构上满足它。
 */
export interface UnmergedInfo {
  /** 只在这个分支上、不在 `comparedTo` 上的提交数。 */
  unmerged: number
  /** 上面那个数是拿哪个 ref 比出来的，如 `main`。 */
  comparedTo: string
  /** 提交对象没进去，但内容已经在 `comparedTo` 上了（squash / rebase 合并）。 */
  contentMerged: boolean
  /**
   * `contentMerged` 是靠哪个已合并 PR 的合并点证明的；分支自己认得出来时为 null。
   * 可选是为了兼容没有这个字段的老后端，缺失与 null 同义。
   */
  mergedVia?: string | null
  /**
   * `contentMerged` 为真时内容实际是在哪个 ref 上找到的。不一定等于 `comparedTo`：
   * 叠 PR 合进的可能是别的分支。可选是为了兼容老后端，缺失时退回 `comparedTo`。
   */
  mergedInto?: string | null
}

/** 那一行的样子：`warning` 是红色警告，`note` 是中性提示。 */
export interface UnmergedNote {
  tone: 'warning' | 'note'
  text: string
}

/**
 * 这个分支上已合并、且带合并点的 PR，编号大的在前。
 *
 * 合并点交给后端当 squash 识别的候选——本地分支落后于合并点时（别处又推了提交再
 * 合并），分支自己的 patch-id 对不上 squash 提交，只有换成合并点去比才认得出。
 * 后端只试前若干个，所以按编号从新到旧排：最近那次合并最可能就是要找的。
 *
 * 分支名精确匹配，不做大小写折叠，与 `indexPullRequestsByBranch` 同一约定。
 */
export function mergedPullRequestsFor(prs: PullRequest[], branch: string): PullRequest[] {
  if (!branch) return []
  return prs
    .filter((pr) => pr.state === 'MERGED' && pr.headRefName === branch && pr.headRefOid)
    .sort((a, b) => b.number - a.number)
}

/**
 * 「该分支有 N 个提交不在 X 上」那一行该怎么说。没有未合并提交时返回 null，什么都不显示。
 *
 * 同一个数字三种读法：
 * - 内容也没进去 → 红色警告，删了只能靠 reflog 找回；
 * - 分支自己认得出 squash → 中性提示，删掉不丢东西；
 * - 靠 PR 合并点认出来的 → 中性提示，并说清是哪个 PR、合并点比本地新。不说的话
 *   「改动已经进去了」和本地分支的实际内容对不上号，用户无从核对。
 */
export function unmergedNote(info: UnmergedInfo, mergedPrs: PullRequest[]): UnmergedNote | null {
  if (info.unmerged <= 0) return null
  const lead = `该分支有 ${info.unmerged} 个提交不在 ${info.comparedTo} 上`
  if (!info.contentMerged) {
    return { tone: 'warning', text: `${lead}，删除后只能靠 reflog 找回。` }
  }
  const via = info.mergedVia
  if (!via) {
    return {
      tone: 'note',
      text: `${lead}，但改动已经进去了（squash 或 rebase 合并后的正常现象），删掉不会丢东西。`,
    }
  }
  const short = via.slice(0, 7)
  const pr = mergedPrs.find((p) => p.headRefOid.toLowerCase() === via.toLowerCase())
  const where = pr
    ? `已合并的 PR #${pr.number} 里（合并的是 ${short}，比本地分支新）`
    : `已合并 PR 的合并点 ${short} 里（比本地分支新）`
  return {
    tone: 'note',
    text: `${lead}，但都包含在${where}，改动已经进了 ${info.mergedInto || info.comparedTo}，删掉不会丢东西。`,
  }
}
