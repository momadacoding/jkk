import { describe, expect, it } from 'vitest'
import { nextFocusIndex } from '@/engine/focusTrap'

// 弹框里 Tab / Shift+Tab 只在框内循环（2026-10-22 E2E：Tab 会把焦点移到弹框后面的主界面）
describe('nextFocusIndex', () => {
  it('Tab 往后一个，最后一个之后回到第一个', () => {
    expect(nextFocusIndex(3, 0, false)).toBe(1)
    expect(nextFocusIndex(3, 2, false)).toBe(0)
  })

  it('Shift+Tab 往前一个，第一个之前回到最后一个', () => {
    expect(nextFocusIndex(3, 2, true)).toBe(1)
    expect(nextFocusIndex(3, 0, true)).toBe(2)
  })

  it('焦点不在框里（比如数据还没读回来、按钮都没聚焦）：Tab 进第一个，Shift+Tab 进最后一个', () => {
    expect(nextFocusIndex(3, -1, false)).toBe(0)
    expect(nextFocusIndex(3, -1, true)).toBe(2)
  })

  it('只有一个可聚焦元素：停在它身上', () => {
    expect(nextFocusIndex(1, 0, false)).toBe(0)
    expect(nextFocusIndex(1, 0, true)).toBe(0)
  })

  it('框里没有可聚焦的（按钮全禁用）：哪也不去，也不能放焦点出框', () => {
    expect(nextFocusIndex(0, -1, false)).toBeNull()
    expect(nextFocusIndex(0, -1, true)).toBeNull()
  })
})
