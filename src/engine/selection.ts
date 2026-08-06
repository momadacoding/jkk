/**
 * 决定一次刷新之后该选中哪个提交。
 *
 * 默认「不打扰」：已经有选中就保持不动。后台文件监听每次仓库变动都会刷图，
 * 不能把用户正在看的提交拽走。
 *
 * `force` 用于 pull / checkout 这类**由用户主动点出来、且移动了 HEAD** 的操作
 * ——这时选中要跟到新 HEAD 上，否则拉回一堆新提交，图却停在原地。
 *
 * 抽成纯函数是为了可测：本仓库没有 jsdom，store 是个 React hook 跑不起来。
 *
 * @param current 当前选中的提交 id
 * @param ids 图里所有提交的 id，顺序即显示顺序
 * @param preferred 希望选中的提交，通常是新 HEAD
 * @param force preferred 是否压过 current
 */
export function nextSelection(
  current: string | null,
  ids: string[],
  preferred?: string,
  force = false,
): string | null {
  // 历史有 5000 条上限，preferred 可能被截断在图外。选一个图里没有的 id
  // 会留下悬空的 selectedId——详情面板空白且毫无解释，所以先确认它在图上。
  const available = preferred && ids.includes(preferred) ? preferred : undefined
  if (force && available) return available
  return current ?? available ?? ids[0] ?? null
}
