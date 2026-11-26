import type { CommitNode } from './types'

type Times = Pick<CommitNode, 'time' | 'authorTime'>

/**
 * 给人看的时间：写下提交的时间（作者时间），和 `git log`、文件历史、blame 一致。
 *
 * 不能直接显示 `time`：那是提交时间，跟进、rebase、cherry-pick、amend 重放时都改成当时。
 * jkk 每次跟进都重放分支上的提交，11-09 写的补丁会显示成跟进那一刻（2026-11-26 报告）。
 * 后端只在两个时间不同时带 `authorTime`；stash、svn 节点本来就没有，都退回 `time`。
 *
 * 只用于显示。排序、图布局、标签排序继续用 `time`，换了会挪动 svn 节点的插入位置。
 */
export function shownTime(commit: Times): number {
  return commit.authorTime ?? commit.time
}

/** 提交被重放改写过时，最后一次的提交时间；没改写过（两个时间相同或没有作者时间）返回 null。 */
export function rewrittenTime(commit: Times): number | null {
  return commit.authorTime !== undefined && commit.authorTime !== commit.time ? commit.time : null
}

/**
 * 日期的悬停提示：只在被重放过时才有，说清楚这里显示的是哪个时间、另一个是什么。
 *
 * 列表按提交时间排序、显示作者时间，重放过的提交在日期列上会「不按顺序」，
 * 不说明的话只能点开详情才知道为什么。没重放过时不给提示：stash 这类节点只有一个时间，
 * 硬说它是「作者时间」反而不对。
 */
export function rewrittenTitle(commit: Times): string | undefined {
  const t = rewrittenTime(commit)
  if (t === null) return undefined
  return `写下提交的时间（作者时间）\n提交时间：${new Date(t * 1000).toLocaleString()}（跟进、rebase、cherry-pick、amend 会改成当时）`
}
