/**
 * 弹框里按 Tab / Shift+Tab 之后焦点该落到第几个可聚焦元素。
 *
 * `current` 为 -1 表示焦点不在框里（比如框里的按钮还没聚焦、或焦点已经漏到了背后）：
 * Tab 进第一个，Shift+Tab 进最后一个。返回 null 表示框里没有可聚焦的元素
 * （执行中按钮全禁用）——此时焦点哪也不去，但同样不能放它出框。
 */
export function nextFocusIndex(count: number, current: number, backward: boolean): number | null {
  if (count === 0) return null
  if (current < 0) return backward ? count - 1 : 0
  return backward ? (current - 1 + count) % count : (current + 1) % count
}
