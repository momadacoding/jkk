import { useCallback, useEffect, useState } from 'react'
import { confirm } from '@tauri-apps/plugin-dialog'
import type { UpstreamInfo } from '../engine/types'
import { planToolbarAction, type ToolbarAction } from '../engine/toolbarActions'

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

/**
 * 弹一个确认框，返回用户是否点了确定。
 *
 * 必须用插件导出的 `confirm` 并 await 它：Tauri 的 dialog 插件把全局
 * `window.confirm` 换成了 async 函数，`!window.confirm(...)` 拿到的是 Promise
 * ——恒为真值，守卫会静默失效，强推照推不误。
 *
 * 弹框本身失败（权限没开、插件没注册）时一律当作「没确认」：
 * 这是唯一不可逆的动作，宁可什么都不做，也不能在没问过的情况下推上去。
 */
async function askConfirm(message: string): Promise<boolean> {
  try {
    return await confirm(message, { title: 'Force push', kind: 'warning' })
  } catch (err) {
    console.error('确认框弹不出来，已按「取消」处理：', err)
    return false
  }
}

interface ToolbarProps {
  /** 当前分支名，来自 state.head。 */
  branch: string
  upstream: UpstreamInfo | null
  /** 执行一次工具栏动作；由 RepoTab 负责 invoke 与刷新。 */
  onRun: (command: string, args: Record<string, unknown>) => Promise<void>
}

const ACTIONS: ToolbarAction[] = ['fetch', 'pull', 'push', 'stash']

export function Toolbar({ branch, upstream, onRun }: ToolbarProps) {
  const ctrl = useCtrlHeld()
  const [busy, setBusy] = useState<ToolbarAction | null>(null)

  const run = useCallback(async (action: ToolbarAction) => {
    const plan = planToolbarAction(action, ctrl, branch)
    if (plan.confirm && !(await askConfirm(plan.confirm))) return
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
      {ACTIONS.map((action) => {
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
                {action === 'pull' ? '↓' : '↑'}{count}
              </span>
            )}
          </button>
        )
      })}
    </div>
  )
}
