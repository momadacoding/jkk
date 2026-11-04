import type { ConflictInfo, FileChange } from './types'

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

/** 暂存前的冲突标记确认框文案。纯逻辑，组件只负责弹出来。 */
export interface StageConflictConfirmModel {
  title: string
  message: string
  confirmLabel: string
  danger: true
  focusCancel: true
}

function listPaths(heading: string, paths: string[]): string {
  return [heading, ...paths.map((p) => `  ${p}`)].join('\n')
}

/**
 * 要暂存的路径里有还留着冲突标记的冲突文件时，返回确认框文案；不用拦时返回 null。
 *
 * git add 不检查冲突标记，暂存即「标记为已解决」，紧接着一次 commit 标记就进了历史
 * （第 85 条）。所以所有 Stage 入口都要先过这里。
 *
 * 判据只用 `read_conflicts` 已有的分类，不在前端重新扫文件：
 * - `markers`：拦
 * - 冲突信息还没读到（`conflicts` 为 null），或读到的那份里没有这个文件
 *   （status 比冲突信息新）：也拦。没读到不等于没有标记，放行就是把未知当安全
 * - `noMarkers`、二进制冲突、一边删一边改、结构冲突：不拦，冲突说明里已给出处理方式
 *
 * 确认框是整批的：取消后这一批都不暂存，不会只暂存其中的普通文件。默认按钮是取消。
 *
 * @param paths 这次要暂存的全部路径
 * @param unstaged 当前 Unstaged 区（冲突文件只会出现在这里）
 * @param conflicts 冲突分类；null 表示还没读到
 */
export function describeStageConflicts(
  paths: string[],
  unstaged: FileChange[],
  conflicts: ConflictInfo[] | null,
): StageConflictConfirmModel | null {
  if (paths.length === 0) return null
  // Set / Map：Stage All 几万个文件时 includes / find 是平方级，弹框前会卡住界面
  const picked = new Set(paths)
  const byPath = new Map((conflicts ?? []).map((c) => [c.path, c]))
  const markers: string[] = []
  const unknown: string[] = []
  for (const f of unstaged) {
    if (f.status !== 'conflicted' || !picked.has(f.path)) continue
    const info = byPath.get(f.path)
    if (!info) unknown.push(f.path)
    else if (info.case === 'markers') markers.push(f.path)
  }
  const flagged = markers.length + unknown.length
  if (flagged === 0) return null

  const title = flagged === 1
    ? markers.length === 1
      ? `${markers[0]} 里还有冲突标记`
      : `无法确认 ${unknown[0]} 里有没有冲突标记`
    : `${flagged} 个文件可能还有冲突标记`
  const sections = [
    '暂存即标记为已解决，冲突标记会随下一次提交进入历史。仍然暂存？',
    markers.length > 0 ? listPaths(`还有冲突标记（${markers.length}）：`, markers) : null,
    unknown.length > 0 ? listPaths(`还没读到冲突信息，无法确认（${unknown.length}）：`, unknown) : null,
    // 不说「选中的」：Stage All 并没有选中什么
    paths.length > flagged ? `取消的话，这一批 ${paths.length} 个文件都不暂存。` : null,
  ]
  return {
    title,
    message: sections.filter((s): s is string => s !== null).join('\n\n'),
    confirmLabel: '仍然暂存',
    danger: true,
    focusCancel: true,
  }
}
