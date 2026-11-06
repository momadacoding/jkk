import { useState, type ReactNode } from 'react'
import type { SidebarSectionKey } from '@/engine/layoutMemory'
import { loadSectionOpen, saveSectionOpen } from '@/store/layoutStore'

interface SidebarSectionProps {
  title: string
  /** 显示在标题右边的计数。0 也会显示，用来说明「确实是空的」。 */
  count?: number
  /** 默认是否展开。传了 `memoryKey` 时只在用户从没点过这个分区时生效。 */
  defaultOpen?: boolean
  /** 记住展开状态用的键；不传就不记，每次挂载都回到 `defaultOpen`。 */
  memoryKey?: SidebarSectionKey
  /**
   * 标题栏最右侧的按钮，通常是「新建」。
   *
   * 点它不应该顺带折叠分区，所以内部会挡掉冒泡。
   */
  action?: { title: string; onClick: () => void }
  children: ReactNode
}

/**
 * 展开状态：初值优先取记忆，切换时写回记忆。
 *
 * 只在挂载时读一次——同一个分区在别的仓库标签里被点了，不去改这里已经显示出来的状态，
 * 免得用户眼前的侧边栏无故跳动；下次挂载自然会读到最新值。
 */
function useSectionOpen(memoryKey: SidebarSectionKey | undefined, defaultOpen: boolean) {
  const [open, setOpen] = useState(() =>
    memoryKey ? loadSectionOpen(memoryKey, defaultOpen) : defaultOpen,
  )
  // 用函数式更新：同一批里连点两下时，两次都要基于最新值翻转。
  // 写存档放在 updater 里是有意的——它是幂等的覆盖写，StrictMode 重复调用也无害
  const toggle = () => {
    setOpen((cur) => {
      const next = !cur
      if (memoryKey) saveSectionOpen(memoryKey, next)
      return next
    })
  }
  return [open, toggle] as const
}

/**
 * 侧边栏的一个可折叠分区。
 *
 * 分支多起来之后 Branches 会把 Worktrees 和 Stashes 挤到屏幕外——能折叠才够用。
 */
export function SidebarSection({ title, count, defaultOpen = true, memoryKey, action, children }: SidebarSectionProps) {
  const [open, toggle] = useSectionOpen(memoryKey, defaultOpen)

  return (
    <div className={open ? 'sb-section' : 'sb-section collapsed'}>
      <div className="sb-section-header" onClick={toggle}>
        <span className="sb-caret">{open ? '▾' : '▸'}</span>
        <span className="sb-section-title">{title}</span>
        {count !== undefined && <span className="sb-count">{count}</span>}
        {action && (
          <button
            className="sb-section-action"
            title={action.title}
            onClick={(e) => { e.stopPropagation(); action.onClick() }}
          >
            +
          </button>
        )}
      </div>
      {open && <div className="sb-section-body">{children}</div>}
    </div>
  )
}

interface SidebarGroupProps {
  label: string
  count: number
  defaultOpen?: boolean
  /** 同 `SidebarSection.memoryKey`。 */
  memoryKey?: SidebarSectionKey
  children: ReactNode
}

/** 分区内部的子分组（Local / Remote），比分区标题低一级。 */
export function SidebarGroup({ label, count, defaultOpen = true, memoryKey, children }: SidebarGroupProps) {
  const [open, toggle] = useSectionOpen(memoryKey, defaultOpen)
  return (
    <div className="sb-group">
      <div className="sb-group-header" onClick={toggle}>
        <span className="sb-caret">{open ? '▾' : '▸'}</span>
        <span>{label}</span>
        <span className="sb-count">{count}</span>
      </div>
      {open && children}
    </div>
  )
}
