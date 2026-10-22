import { useEffect, type RefObject } from 'react'
import { nextFocusIndex } from '../engine/focusTrap'

const FOCUSABLE = [
  'button:not([disabled])',
  'input:not([disabled])',
  'textarea:not([disabled])',
  'select:not([disabled])',
  'a[href]',
  '[tabindex]:not([tabindex="-1"])',
].join(', ')

/** 当前开着的弹框，后打开的在最后；只有最上面那层管 Tab。 */
const open: RefObject<HTMLElement | null>[] = []

/**
 * 把 Tab / Shift+Tab 圈在弹框里。
 *
 * 弹框盖住了主界面，但浏览器的 Tab 顺序不管遮罩：不圈的话焦点会走到背后的菜单栏、
 * 工具栏按钮上，这时按回车就点到了背后的 Push 之类（2026-10-22 E2E 第 7 条复验时发现）。
 *
 * 每次 Tab 都由这里挪焦点（不只在首尾）：焦点已经漏到框外、或框里没有可聚焦元素时，
 * 浏览器默认的下一个落点都在框外，只拦首尾拦不住。监听挂在 window 的捕获阶段，
 * 焦点在框外时也收得到。
 */
export function useFocusTrap(ref: RefObject<HTMLElement | null>) {
  useEffect(() => {
    open.push(ref)
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Tab' || open[open.length - 1] !== ref) return
      const box = ref.current
      if (!box) return
      e.preventDefault()
      const items = Array.from(box.querySelectorAll<HTMLElement>(FOCUSABLE))
      const current = items.indexOf(document.activeElement as HTMLElement)
      const next = nextFocusIndex(items.length, current, e.shiftKey)
      if (next !== null) items[next].focus()
    }
    window.addEventListener('keydown', onKey, true)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      open.splice(open.indexOf(ref), 1)
    }
  }, [ref])
}
