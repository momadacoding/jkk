import { useCallback, useEffect, useState } from 'react'
import { askConfirm } from '../engine/confirm'
import type { CommandContext } from '../engine/customCommands'
import type { UpstreamInfo } from '../engine/types'
import {
  planToolbarAction,
  showsSvnDeliver,
  showsSvnUpdate,
  visibleToolbarActions,
  type ToolbarAction,
} from '../engine/toolbarActions'
import type { Capabilities } from '../engine/capabilities'
import { CustomCommandBar } from './CustomCommandBar'

/**
 * 跟踪 Ctrl 是否按下。
 *
 * 必须在 blur 时强制复位：按着 Ctrl 切走窗口再切回来，keyup 事件不会送到
 * 本窗口，按钮会永久卡在变体状态上——用户会在毫不知情的情况下强推。
 */
export function useCtrlHeld(): boolean {
  const [held, setHeld] = useState(false)
  useEffect(() => {
    const down = (e: KeyboardEvent) => { if (e.key === 'Control') setHeld(true) }
    const up = (e: KeyboardEvent) => { if (e.key === 'Control') setHeld(false) }
    const reset = () => setHeld(false)
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    window.addEventListener('blur', reset)
    return () => {
      window.removeEventListener('keydown', down)
      window.removeEventListener('keyup', up)
      window.removeEventListener('blur', reset)
    }
  }, [])
  return held
}

interface ToolbarProps {
  /** 当前分支名，来自 state.head。 */
  branch: string
  upstream: UpstreamInfo | null
  /** 执行一次工具栏动作；由 RepoTab 负责 invoke 与刷新。 */
  onRun: (command: string, args: Record<string, unknown>) => Promise<void>
  /** 自定义命令的占位符取值来源；未打开仓库时 worktree 为 null。 */
  commandContext: CommandContext
  /** 后端能力。不支持远程的后端不渲染 fetch/pull/push。 */
  capabilities: Capabilities
  /** SVN 混合仓库的「更新」（sync_in）。预览、确认、执行都由 RepoTab 编排。 */
  onSvnUpdate?: () => Promise<void>
  /** SVN 混合仓库的「交付」（sync_out）。同上，由 RepoTab 编排。 */
  onSvnDeliver?: () => Promise<void>
}

export function Toolbar({
  branch,
  upstream,
  onRun,
  commandContext,
  capabilities,
  onSvnUpdate,
  onSvnDeliver,
}: ToolbarProps) {
  const ctrl = useCtrlHeld()
  const [busy, setBusy] = useState<ToolbarAction | null>(null)
  const [updating, setUpdating] = useState(false)
  const [delivering, setDelivering] = useState(false)
  // 更新与交付同一时间只走一个（流程层的 running 也拦着，这里让按钮看得出来）
  const svnBusy = updating || delivering

  const run = useCallback(async (action: ToolbarAction) => {
    const plan = planToolbarAction(action, ctrl, branch)
    if (plan.confirm && !(await askConfirm(plan.confirm, 'Force push'))) return
    setBusy(action)
    try {
      await onRun(plan.command, plan.args)
    } finally {
      setBusy(null)
    }
  }, [ctrl, branch, onRun])

  /** Pull 显示落后数、Push 显示领先数；没有上游就不显示。 */
  const badge = (action: ToolbarAction): number => {
    if (!upstream?.upstream) return 0
    if (action === 'pull') return upstream.behind
    if (action === 'push') return upstream.ahead
    return 0
  }

  return (
    <div className="toolbar">
      <span className="toolbar-branch" title={upstream?.upstream ?? 'no upstream'}>
        ⎇ {branch}
      </span>
      <span className="toolbar-sep" />
      {showsSvnUpdate(capabilities) && onSvnUpdate && (
        <button
          className="toolbar-button"
          disabled={busy !== null || svnBusy}
          onClick={() => {
            setUpdating(true)
            void onSvnUpdate().finally(() => setUpdating(false))
          }}
          data-action="svn-update"
        >
          {updating ? '…' : '更新'}
        </button>
      )}
      {showsSvnDeliver(capabilities) && onSvnDeliver && (
        <button
          className="toolbar-button"
          disabled={busy !== null || svnBusy}
          onClick={() => {
            setDelivering(true)
            void onSvnDeliver().finally(() => setDelivering(false))
          }}
          data-action="svn-deliver"
        >
          {delivering ? '…' : '交付'}
        </button>
      )}
      {visibleToolbarActions(capabilities).map((action) => {
        const plan = planToolbarAction(action, ctrl, branch)
        const count = badge(action)
        return (
          <button
            key={action}
            className={`toolbar-button${ctrl ? ' variant' : ''}`}
            disabled={busy !== null}
            onClick={() => void run(action)}
            data-action={action}
          >
            {busy === action ? '…' : plan.label}
            {count > 0 && (
              <span className="toolbar-badge">
                {action === 'pull' ? '↓' : '↑'}&thinsp;{count}
              </span>
            )}
          </button>
        )
      })}
      <span className="toolbar-spacer" />
      <CustomCommandBar context={commandContext} />
    </div>
  )
}
