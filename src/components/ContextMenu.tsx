import { useEffect, useRef, useState } from 'react'

export interface MenuItem {
  label: string
  onSelect: () => void
  /** 置灰但仍然显示，用来说明「为什么这里不能做这件事」。 */
  disabled?: boolean
  /** 危险操作（删除类），显示为红色。 */
  danger?: boolean
  /** 右侧灰字提示的快捷键，只是展示；按键本身由调用方自己处理。 */
  shortcut?: string
}

export interface MenuState {
  x: number
  y: number
  items: MenuItem[]
}

/**
 * 右键菜单。
 *
 * 由调用方持有 `MenuState`（用 `useContextMenu` 更省事），
 * 本组件只负责定位、渲染和关闭。
 */
export function ContextMenu({ menu, onClose }: { menu: MenuState | null; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ x: 0, y: 0 })

  // 贴着窗口边缘弹出时会被截断，量完自身尺寸再回收进可视区域
  useEffect(() => {
    if (!menu) return
    const el = ref.current
    if (!el) return
    const { width, height } = el.getBoundingClientRect()
    setPos({
      x: Math.min(menu.x, window.innerWidth - width - 4),
      y: Math.min(menu.y, window.innerHeight - height - 4),
    })
  }, [menu])

  useEffect(() => {
    if (!menu) return
    // 捕获阶段监听是为了抢在下层行的 handler 之前关掉菜单，
    // 但菜单自身内部的按下必须放行——否则菜单先卸载，按钮的 click 永远不会触发。
    const onMouseDown = (e: MouseEvent) => {
      if (ref.current?.contains(e.target as Node)) return
      onClose()
    }
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    const onResize = () => onClose()
    window.addEventListener('mousedown', onMouseDown, true)
    window.addEventListener('resize', onResize)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onMouseDown, true)
      window.removeEventListener('resize', onResize)
      window.removeEventListener('keydown', onKey)
    }
  }, [menu, onClose])

  if (!menu) return null

  return (
    <div
      ref={ref}
      className="context-menu"
      style={{ left: pos.x, top: pos.y }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {menu.items.map((item, i) => (
        <button
          key={i}
          className={`context-menu-item${item.danger ? ' danger' : ''}`}
          disabled={item.disabled}
          // 先让菜单真的从屏幕上消失，再跑动作。同步执行的话，动作里但凡有一步
          // 卡住主线程（原生 prompt/confirm 就是），React 还没来得及重绘，
          // 菜单就会一直留在弹框后面——曾经的 "New branch from here…" 就是这样。
          onClick={() => { onClose(); setTimeout(() => item.onSelect(), 0) }}
        >
          {item.label}
          {item.shortcut && <span className="context-menu-shortcut">{item.shortcut}</span>}
        </button>
      ))}
    </div>
  )
}

/** 管理一个右键菜单的开关状态。 */
export function useContextMenu() {
  const [menu, setMenu] = useState<MenuState | null>(null)
  const open = (e: React.MouseEvent, items: MenuItem[]) => {
    e.preventDefault()
    e.stopPropagation()
    setMenu({ x: e.clientX, y: e.clientY, items })
  }
  const close = () => setMenu(null)
  return { menu, open, close }
}
