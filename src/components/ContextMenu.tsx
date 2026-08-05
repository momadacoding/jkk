import { useEffect, useRef, useState } from 'react'

export interface MenuItem {
  label: string
  onSelect: () => void
  /** 置灰但仍然显示，用来说明「为什么这里不能做这件事」。 */
  disabled?: boolean
  /** 危险操作（删除类），显示为红色。 */
  danger?: boolean
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
          onClick={() => { onClose(); item.onSelect() }}
        >
          {item.label}
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
