/** 丢弃改动的确认框文案。纯逻辑，组件只负责弹出来。 */

export interface DiscardConfirmModel {
  title: string
  message: string
  warning?: string
  confirmLabel: string
  danger: true
  focusCancel: true
}

function listPaths(heading: string, paths: string[]): string {
  return [heading, ...paths.map((p) => `  ${p}`)].join('\n')
}

/**
 * 未跟踪文件在 git 里没有任何副本，删了就找不回来了，所以它们单独进红条，和「能从 git 恢复」
 * 的分开：用户要能判断这次点下去的代价。
 *
 * 清单不截断，确认框的正文会滚动。默认按钮是 Cancel，回车不会丢东西。
 *
 * @param paths 要丢弃的全部路径
 * @param gone `paths` 里无法恢复的那些
 * @param staged 丢弃的是暂存区的改动
 */
export function describeDiscard(
  paths: string[],
  gone: string[],
  staged: boolean,
): DiscardConfirmModel {
  // Set：全选几万个文件时 includes 是平方级，弹框前会卡住界面
  const goneSet = new Set(gone)
  const recoverable = paths.filter((p) => !goneSet.has(p))
  const target = staged ? ' HEAD' : '暂存区的内容'
  // 代价写在首行：能恢复的清单很长时红条在最下面，而 Discard 按钮贴底、不滚动也点得到
  const head =
    gone.length > 0
      ? `丢弃以下改动？其中 ${gone.length} 个未跟踪文件会永久删除，无法恢复（列在最下面）。`
      : '丢弃以下改动？'
  return {
    title: 'Discard changes',
    message:
      recoverable.length > 0
        ? `${head}\n\n${listPaths(`恢复到${target}（${recoverable.length}）：`, recoverable)}`
        : head,
    warning: gone.length > 0 ? listPaths(`永久删除，无法恢复（${gone.length}）：`, gone) : undefined,
    confirmLabel: 'Discard',
    danger: true,
    focusCancel: true,
  }
}
