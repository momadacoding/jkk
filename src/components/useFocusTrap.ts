import { useEffect, useRef, type RefObject } from 'react'
import { nextFocusIndex } from '../engine/focusTrap'

const FOCUSABLE = [
  'button:not([disabled])',
  'input:not([disabled])',
  'textarea:not([disabled])',
  'select:not([disabled])',
  'a[href]',
  '[tabindex]:not([tabindex="-1"])',
].join(', ')

/**
 * 焦点在按钮行（`.dialog-actions`）的按钮上时，← / → 在这一行的按钮之间挪，到头绕回。
 *
 * 系统对话框都这么走，用户的手也这么习惯（2026-11-08 丢弃确认框实测方向键不动）。
 * 只认按钮行里的按钮：焦点在输入框里时方向键要挪光标，在正文的滚动区里要能滚。
 */
function moveInActions(box: HTMLElement, e: KeyboardEvent) {
  if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return
  const current = document.activeElement
  if (!(current instanceof HTMLButtonElement)) return
  const row = current.closest('.dialog-actions')
  if (!row || !box.contains(row)) return
  const items = Array.from(row.querySelectorAll<HTMLButtonElement>('button:not([disabled])'))
  const next = nextFocusIndex(items.length, items.indexOf(current), e.key === 'ArrowLeft')
  if (next === null) return
  e.preventDefault()
  items[next].focus()
}

/** 当前开着的弹框，后打开的在最后；只有最上面那层管 Tab 和方向键。 */
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
 *
 * 顺带管按钮行里的 ← / →，见 `moveInActions`：所有弹框都挂这个 hook，放这里一处就全有。
 */
export function useFocusTrap(ref: RefObject<HTMLElement | null>) {
  // 打开前的焦点在**首次渲染时**记下：effect 里再读就晚了，`autoFocus` 在提交阶段已经把它挪进框里
  const before = useRef<Element | null | undefined>(undefined)
  if (before.current === undefined) before.current = document.activeElement

  useEffect(() => {
    open.push(ref)
    const onKey = (e: KeyboardEvent) => {
      if (open[open.length - 1] !== ref) return
      const box = ref.current
      if (!box) return
      if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
        moveInActions(box, e)
        return
      }
      if (e.key !== 'Tab') return
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
      // 关框后把焦点还回去：不还的话落到 body，文件列表的 Del / Ctrl+A 就收不到了。
      // 焦点已被别处接走（比如紧接着弹出的下一个框）时不抢
      const prev = before.current
      const lost = !document.activeElement || document.activeElement === document.body
      if (lost && prev instanceof HTMLElement && prev.isConnected) prev.focus()
    }
  }, [ref])
}
