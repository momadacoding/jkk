import type { CommitNode } from './types'
import { WORKDIR_ID } from './owners'

export { WORKDIR_ID }

/**
 * 在提交列表最前面插入工作区伪行。
 *
 * 它是个合成提交，第一父指向 HEAD，于是布局时天然落在 HEAD 那条提交的
 * 正上方；`computeLayout` 会把它排除在归属与装箱之外，只让它继承父的车道。
 * 空仓库或游离状态取不到 HEAD 时不插，图上就没有这一行。
 */
export function withWorkdirRow(commits: CommitNode[], headHash: string | null): CommitNode[] {
  if (!headHash) return commits
  return [
    { id: WORKDIR_ID, parents: [headHash], author: '', time: 0, message: '' },
    ...commits,
  ]
}
