import type { FileChange } from './types'

/**
 * 冲突文件排到最前，其余保持原顺序。
 *
 * 冲突文件和普通改动一起按路径混排时，改动一多就淹没在列表中间，
 * 而它们恰恰是 merge / rebase 停下来之后唯一必须先处理的东西。
 * 没有冲突时返回原数组，免得每次刷新都换一个新引用。
 */
export function conflictsFirst(files: FileChange[]): FileChange[] {
  if (!files.some((f) => f.status === 'conflicted')) return files
  return [
    ...files.filter((f) => f.status === 'conflicted'),
    ...files.filter((f) => f.status !== 'conflicted'),
  ]
}

export function countConflicts(files: FileChange[]): number {
  return files.filter((f) => f.status === 'conflicted').length
}

/**
 * 顶部横幅文案；`null` 表示不显示。
 *
 * 只说「merge in progress」不说还剩几个冲突，用户得自己去列表里找——
 * 横幅本来就是最显眼的位置，把数字放在这里。
 */
export function repoStateBanner(state: string, conflicts: number): string | null {
  if (state === 'normal') return null
  return conflicts > 0
    ? `${state} in progress · ${conflicts} 个文件有冲突`
    : `${state} in progress`
}
