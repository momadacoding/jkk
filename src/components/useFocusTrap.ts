import { useEffect, useLayoutEffect, useRef, type RefObject } from 'react'
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
  // 与 Tab 认的可聚焦元素一致：tabindex=-1 的按钮 focus() 不一定落得上，落不上方向键就被白吃
  const items = Array.from(
    row.querySelectorAll<HTMLButtonElement>('button:not([disabled]):not([tabindex="-1"])'),
  )
  const next = nextFocusIndex(items.length, items.indexOf(current), e.key === 'ArrowLeft')
  if (next === null) return
  e.preventDefault()
  items[next].focus()
}

/** 当前开着的弹框，后打开的在最后；只有最上面那层管 Tab、方向键和 Esc。 */
const open: RefObject<HTMLElement | null>[] = []

/**
 * 元素是不是真的显示着：自己和祖先都没有 `display: none`。
 *
 * App 让所有标签保持挂载、非活动的整个 `display:none`，藏起来的标签里照样可能开着弹框
 * （后台跑完的跟进弹出结局）。这种框看不见，不能拦当前标签的按键（PR 145 review 第 2、3 条）。
 * 用 computed style 而不是 offsetParent：弹框遮罩是 fixed 定位，offsetParent 恒为 null。
 */
export function isShown(el: Element | null): boolean {
  if (!el?.isConnected) return false
  for (let n: Element | null = el; n; n = n.parentElement) {
    if (getComputedStyle(n).display === 'none') return false
  }
  return true
}

/** 看得见的弹框里最上面的那个。 */
function topShown(): RefObject<HTMLElement | null> | undefined {
  for (let i = open.length - 1; i >= 0; i--) {
    if (isShown(open[i].current)) return open[i]
  }
  return undefined
}

/**
 * 有没有看得见的弹框开着。挂在 window 上的全局快捷键（graph 的 ↑ / ↓）据此让路：弹框盖住了主界面，
 * 背后的选中还在跟着按键变，关框之后看到的就不是弹框说的那一条了（PR 139 review 第 6 条）。
 */
export function anyDialogOpen(): boolean {
  return topShown() !== undefined
}

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
 *
 * 也管鼠标按在框外：遮罩不关框之后，点它的默认动作会把焦点从框里的按钮挪到 body，
 * 框还开着，回车、方向键却都没了着落（2026-11-08 PR review 第 1 条）。框外的 mousedown
 * 一律取消默认动作，焦点留在原处；click 照常派发，框外的右键菜单之类不受影响。
 *
 * `onEscape`：按 Esc 时调用，只调最上面那层的。各框原来各自往 window 上挂 Esc 监听，
 * 叠两层框时一次 Esc 两层一起关（PR 139 review 第 5 条）。这条挂在**冒泡**阶段：框里的控件
 * 自己吃掉 Esc（设置框录快捷键时 stopPropagation）就到不了这里，不会顺手把框关了。
 * 回调取最近一次**提交**的渲染里的那个，「执行中不许关」这类判断写在回调里即可。
 * 按住 Esc 的自动重复、输入法组字中的 Esc 都不算：前者会在上层关掉后接着关下层，
 * 后者是在取消候选词。
 *
 * 「最上面那层」只在看得见的框里挑，见 `isShown`。
 */
export function useFocusTrap(ref: RefObject<HTMLElement | null>, onEscape?: () => void) {
  // 打开前的焦点在**首次渲染时**记下：effect 里再读就晚了，`autoFocus` 在提交阶段已经把它挪进框里
  const before = useRef<Element | null | undefined>(undefined)
  if (before.current === undefined) before.current = document.activeElement
  // 在提交阶段更新：渲染期间赋值的话，被 React 丢弃的那次渲染（并发渲染、StrictMode 双渲染）
  // 也会把它的闭包留下，Esc 可能拿着没生效的 busy 判断去关框
  const escape = useRef(onEscape)
  useLayoutEffect(() => {
    escape.current = onEscape
  })

  useEffect(() => {
    open.push(ref)
    const onKey = (e: KeyboardEvent) => {
      if (topShown() !== ref) return
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
    const onMouseDown = (e: MouseEvent) => {
      if (topShown() !== ref) return
      const box = ref.current
      if (box && e.target instanceof Node && !box.contains(e.target)) e.preventDefault()
    }
    const onEsc = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.repeat || e.isComposing || topShown() !== ref) return
      escape.current?.()
    }
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('keydown', onEsc)
    window.addEventListener('mousedown', onMouseDown, true)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('keydown', onEsc)
      window.removeEventListener('mousedown', onMouseDown, true)
      open.splice(open.indexOf(ref), 1)
      // 关框后把焦点还回去：不还的话落到 body，文件列表的 Del / Ctrl+A 就收不到了。
      // 焦点已被别处接走（比如紧接着弹出的下一个框）时不抢
      const prev = before.current
      const lost = !document.activeElement || document.activeElement === document.body
      if (lost && prev instanceof HTMLElement && prev.isConnected) prev.focus()
    }
  }, [ref])
}
