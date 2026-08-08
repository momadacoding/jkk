import { useState } from 'react'
import type { Worktree } from '../engine/types'
import { samePath } from '../engine/path'
import { ContextMenu, useContextMenu } from './ContextMenu'
import { SidebarSection } from './SidebarSection'
import { DeleteWorktreeDialog } from './DeleteWorktreeDialog'

interface WorktreeViewProps {
  worktrees: Worktree[]
  /** 当前标签打开的仓库路径。 */
  currentPath: string
  /** 已在某个标签里打开的路径，用来提示「已打开」。 */
  openPaths: string[]
  onOpenInNewTab: (path: string) => void
  /**
   * 收完整路径而不是名字——名字得靠拼，从主仓库删会拼到仓库外面去。
   *
   * Promise 要等删完才 resolve，失败要抛出：确认框据此留在原地显示错误，
   * 并把「有未提交改动」升级成「仍要删除」。
   */
  onDelete: (worktreePath: string, force: boolean) => Promise<void>
  onCreate: (name: string) => void
  /** 单击：把 graph 定位到该 worktree 的 HEAD。传入完整 oid。 */
  onSelect: (headHash: string, name: string) => void
  /** 当前被单击选中的 worktree 路径。 */
  selectedPath: string | null
}

function WorktreeIcon() {
  return (
    <svg className="sb-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <rect x="2.5" y="3.5" width="11" height="9" rx="1.5" />
      <path d="M2.5 6.5h11" />
    </svg>
  )
}

export function WorktreeView({
  worktrees, currentPath, openPaths, onOpenInNewTab, onDelete, onCreate, onSelect, selectedPath,
}: WorktreeViewProps) {
  const { menu, open, close } = useContextMenu()
  /** 正在确认删除的 worktree。 */
  const [deleting, setDeleting] = useState<Worktree | null>(null)

  return (
    <SidebarSection
      title="WORKTREES"
      count={worktrees.length}
      action={{ title: 'New worktree', onClick: () => onCreate(`wt-${Date.now()}`) }}
    >
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
            onSelect: () => setDeleting(wt),
            disabled: !canDelete,
            danger: true,
          },
        ]
        return (
          <div
            key={wt.path}
            className={[
              'sb-row',
              isCurrent ? 'current' : '',
              samePath(wt.path, selectedPath ?? '') ? 'selected' : '',
            ].filter(Boolean).join(' ')}
            title={`${wt.path}\n单击定位到它的 HEAD · ${isOpen ? '双击切到它的标签页' : '双击在新标签页打开'} · 右键更多操作`}
            // 单击只定位，不碰工作区——与分支列表保持一致的交互
            onClick={() => onSelect(wt.head, wt.name)}
            // 不加 isOpen 判断：App 的 openInNewTab 本来就会「已开就切过去」，
            // 在这里拦一道只会让双击已打开的 worktree 毫无反应
            onDoubleClick={() => onOpenInNewTab(wt.path)}
            onContextMenu={(e) => open(e, items)}
          >
            <WorktreeIcon />
            <span className="sb-label">{wt.name}</span>
            {wt.branch && <span className="sb-sub">{wt.branch}</span>}
            {/* 行内操作平时不占视觉，hover 才出现 */}
            <span className="sb-actions">
              <button
                title={isOpen ? '切到它的标签页' : '在新标签页打开'}
                onClick={(e) => { e.stopPropagation(); onOpenInNewTab(wt.path) }}
              >
                ↗
              </button>
              <button
                className="danger"
                title={canDelete ? '删除这个 worktree' : wt.isMain ? '主工作区不能删' : '当前打开的工作区不能删'}
                disabled={!canDelete}
                onClick={(e) => { e.stopPropagation(); setDeleting(wt) }}
              >
                ✕
              </button>
            </span>
            {isCurrent && <span className="sb-current-dot" title="当前工作区" />}
          </div>
        )
      })}
      <ContextMenu menu={menu} onClose={close} />
      {deleting && (
        <DeleteWorktreeDialog
          worktree={deleting}
          onCancel={() => setDeleting(null)}
          // 成功才关；失败时 onDelete 抛出，确认框自己接住并留在原地
          onConfirm={async (force) => { await onDelete(deleting.path, force); setDeleting(null) }}
        />
      )}
    </SidebarSection>
  )
}
