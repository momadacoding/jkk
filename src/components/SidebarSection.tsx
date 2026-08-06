import { useState, type ReactNode } from 'react'

interface SidebarSectionProps {
  title: string
  /** 显示在标题右边的计数。0 也会显示，用来说明「确实是空的」。 */
  count?: number
  /** 默认是否展开。 */
  defaultOpen?: boolean
  /**
   * 标题栏最右侧的按钮，通常是「新建」。
   *
   * 点它不应该顺带折叠分区，所以内部会挡掉冒泡。
   */
  action?: { title: string; onClick: () => void }
  children: ReactNode
}

/**
 * 侧边栏的一个可折叠分区。
 *
 * 分支多起来之后 Branches 会把 Worktrees 和 Stashes 挤到屏幕外——能折叠才够用。
 */
export function SidebarSection({ title, count, defaultOpen = true, action, children }: SidebarSectionProps) {
  const [open, setOpen] = useState(defaultOpen)

  return (
    <div className={open ? 'sb-section' : 'sb-section collapsed'}>
      <div className="sb-section-header" onClick={() => setOpen((v) => !v)}>
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
  children: ReactNode
}

/** 分区内部的子分组（Local / Remote），比分区标题低一级。 */
export function SidebarGroup({ label, count, defaultOpen = true, children }: SidebarGroupProps) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <div className="sb-group">
      <div className="sb-group-header" onClick={() => setOpen((v) => !v)}>
        <span className="sb-caret">{open ? '▾' : '▸'}</span>
        <span>{label}</span>
        <span className="sb-count">{count}</span>
      </div>
      {open && children}
    </div>
  )
}
