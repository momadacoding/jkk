import type { Worktree } from '../engine/types'
import { samePath } from '../engine/path'
import { ContextMenu, useContextMenu } from './ContextMenu'

interface WorktreeViewProps {
  worktrees: Worktree[]
  /** 当前标签打开的仓库路径。 */
  currentPath: string
  /** 已在某个标签里打开的路径，用来提示「已打开」。 */
  openPaths: string[]
  onOpenInNewTab: (path: string) => void
  onDelete: (name: string) => void
  onCreate: (name: string) => void
}

export function WorktreeView({
  worktrees, currentPath, openPaths, onOpenInNewTab, onDelete, onCreate,
}: WorktreeViewProps) {
  const { menu, open, close } = useContextMenu()

  return (
    <div className="sidebar-panel">
      <h3>Worktrees</h3>
      {worktrees.map((wt) => {
        const isCurrent = samePath(wt.path, currentPath)
        const isOpen = openPaths.some((p) => samePath(p, wt.path))
        // 主工作区不能删；当前打开的工作区也删不掉（git worktree remove 会拒绝）
        const canDelete = !wt.isMain && !isCurrent
        const items = [
          {
            // 已经开着就切过去，而不是置灰——「已打开」不该是个死胡同
            label: isOpen ? 'Switch to its tab' : 'Open in new tab',
            onSelect: () => onOpenInNewTab(wt.path),
          },
          {
            label: 'Delete worktree',
            onSelect: () => onDelete(wt.name),
            disabled: !canDelete,
            danger: true,
          },
        ]
        return (
          <div
            key={wt.path}
            className={isCurrent ? 'worktree-row active' : 'worktree-row'}
            title={isOpen ? '双击切到它的标签页 · 右键更多操作' : '双击在新标签页打开 · 右键更多操作'}
            // 不加 isOpen 判断：App 的 openInNewTab 本来就会「已开就切过去」，
            // 在这里拦一道只会让双击已打开的 worktree 毫无反应
            onDoubleClick={() => onOpenInNewTab(wt.path)}
            onContextMenu={(e) => open(e, items)}
          >
            <span className="worktree-name">
              {wt.name} {wt.branch && `(${wt.branch})`}
            </span>
            {isOpen && !isCurrent && <span className="worktree-badge">open</span>}
          </div>
        )
      })}
      <button onClick={() => onCreate(`wt-${Date.now()}`)}>+ New worktree</button>
      <ContextMenu menu={menu} onClose={close} />
    </div>
  )
}
