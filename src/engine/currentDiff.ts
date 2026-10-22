import type { DiffData } from './types'

/**
 * 只有按当前视图要的 scope 读出来的 diff 才能显示；否则返回 null，界面说「正在读取」。
 *
 * store 里只有一份 diff，换选中或切视图时新 diff 回来之前它还是上一个 scope 的。
 * 平时几毫秒就换掉了看不出来；5 万文件的提交要几十秒，提交详情就把工作区的改动
 * 当成了这个提交的改动（2026-10-22 E2E）。`wanted` 为 null 表示当前视图不读 diff。
 */
export function currentDiff(diff: DiffData, diffScope: string | null, wanted: string | null): DiffData | null {
  return wanted !== null && diffScope === wanted ? diff : null
}
