import { useCallback, useEffect, useRef, useState } from 'react'
import { toErrorMessage } from '../engine/errors'

/**
 * 结局提示的分量：`attention` 是还要用户动手，`ok` 只是告知。
 * 不从哪个功能模块引类型：这是通用对话框，SVN 的 `OutcomeNotice.tone` 恰好与它同形。
 */
export type NoticeTone = 'ok' | 'attention'

export interface ConfirmRequest {
  title: string
  /** 标题下面那行说明。 */
  message: string
  /** 一行「标签 + 值」，比如 `Branch: origin/feat/x`；值前面画一个图标。 */
  detail?: { label: string; value: string }
  /** 红底警告条，用来说明这次操作的代价。 */
  warning?: string
  confirmLabel?: string
  /** 危险操作，确认按钮变红。 */
  danger?: boolean
  /**
   * 点确认后要跑的活。传了的话弹框会等它跑完再关，期间显示进度。
   *
   * 涉及网络的操作必须走这条路：`git push --delete` 到 GitHub 实测四五秒，
   * 确认完就关框的话界面上这几秒毫无动静，看着像卡死。抛出异常则留在原地
   * 显示错误，用户可以就地重试。
   */
  action?: () => Promise<void>
  /** 进度文案，配合 `action`。 */
  busyLabel?: string
  /**
   * 设了就是结局提示，不是确认：只有一个按钮（没有可取消的东西），图标按分量选。
   * 不设时照旧是确认框。
   */
  notice?: NoticeTone
}

interface Pending extends ConfirmRequest {
  resolve: (ok: boolean) => void
  /** 每个请求一个，做 `<Dialog>` 的 key，见 `useConfirm`。 */
  id: number
}

/** 弹框头部的警告图标。交付确认框（`DeliverDialog`）也用，两个框长得一样。 */
export function WarnGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" strokeWidth="1.6">
      <path d="M12 4 2.5 20h19z" />
      <path d="M12 10v5M12 17.5v.5" />
    </svg>
  )
}

/** 结局提示 `ok` 的图标：只是告知，不该长得像警告。 */
function CheckGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M5 12.5 10 17.5 19 7.5" />
    </svg>
  )
}

function iconClass(r: ConfirmRequest): string {
  if (r.notice) return `dlg-icon ${r.notice}`
  return r.danger ? 'dlg-icon danger' : 'dlg-icon'
}

function RemoteGlyph() {
  return (
    <svg className="dlg-glyph" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <path d="M4.5 12.5a3 3 0 0 1-.3-6 4 4 0 0 1 7.7-.6 2.8 2.8 0 0 1-.4 6.6z" />
    </svg>
  )
}

function Dialog({ request, onDone }: { request: Pending; onDone: () => void }) {
  const confirmRef = useRef<HTMLButtonElement>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const finish = (ok: boolean) => {
    request.resolve(ok)
    onDone()
  }

  const accept = async () => {
    if (busy) return
    if (!request.action) {
      finish(true)
      return
    }
    setBusy(true)
    setError(null)
    try {
      await request.action()
      finish(true)
    } catch (err) {
      setError(toErrorMessage(err))
      setBusy(false)
    }
  }

  // 焦点必须落进弹框里，回车和 Esc 才有着落
  useEffect(() => { confirmRef.current?.focus() }, [])

  useEffect(() => {
    // 活跑起来之后 Esc 不再关闭，关掉只会让用户以为取消了
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && !busy) finish(false) }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [busy])

  return (
    <div className="dialog-overlay" onMouseDown={() => { if (!busy) finish(false) }}>
      <div className="dialog dialog-wide" onMouseDown={(e) => e.stopPropagation()}>
        <div className="dlg-head">
          <span className={iconClass(request)}>
            {request.notice === 'ok' ? <CheckGlyph /> : <WarnGlyph />}
          </span>
          <div>
            <div className="dialog-title">{request.title}</div>
            <div className="dialog-message">{request.message}</div>
          </div>
        </div>

        {request.detail && (
          <div className="dlg-field">
            <span className="dlg-field-label">{request.detail.label}</span>
            <span className="dlg-value"><RemoteGlyph />{request.detail.value}</span>
          </div>
        )}

        {request.warning && <div className="dlg-warning">{request.warning}</div>}

        {error && <div className="dlg-warning">{error}</div>}

        {busy && (
          <div className="dlg-progress">
            <span className="dlg-spinner" />
            {request.busyLabel ?? '处理中…'}
          </div>
        )}

        <div className="dialog-actions">
          {!request.notice && <button disabled={busy} onClick={() => finish(false)}>Cancel</button>}
          <button
            ref={confirmRef}
            className={request.danger ? 'primary danger' : 'primary'}
            disabled={busy}
            onClick={() => void accept()}
          >
            {busy ? '…' : request.confirmLabel ?? 'OK'}
          </button>
        </div>
      </div>
    </div>
  )
}

/**
 * 应用内的确认框，和 `usePrompt` 同一套用法与观感。
 *
 * 与 `engine/confirm.ts` 的 `askConfirm` 的分工：那个走 Tauri 的系统对话框，
 * 适合纯文本的是/否；这个能显示分支名、图标和警告条，跟删除分支那个框长得一样。
 */
export function useConfirm() {
  const [pending, setPending] = useState<Pending | null>(null)
  const seq = useRef(0)

  const confirm = useCallback(
    (request: ConfirmRequest) =>
      new Promise<boolean>((resolve) => setPending({ ...request, resolve, id: ++seq.current })),
    [],
  )

  // key 不能省：一个框带着 `action` 跑完、紧接着 `await confirm(...)` 弹下一个（更新结束后
  // 弹结局）时，`setPending(null)` 与 `setPending(下一个)` 被 React 18 合并成一次渲染，
  // `<Dialog>` 从没卸载——上一个框的 `busy` 原样带进新框，新框一出来就是「处理中…」、
  // 按钮禁用、Esc 与点遮罩也关不掉（2026-09-28 Phase 2c E2E 场景 2 实测）。
  // 每个请求一个 key，新框就是全新的组件，busy / error / 初始焦点都从头来。
  //
  // 关框只关自己：旧框执行中被新请求顶替时，它的 action 跑完仍会调 onDone，
  // 无条件清空会把新框一起关掉、新框的 Promise 永远不结束（PR #49 Codex review #5）
  const dialog = pending ? (
    <Dialog
      key={pending.id}
      request={pending}
      onDone={() => setPending((p) => (p?.id === pending.id ? null : p))}
    />
  ) : null

  return { confirm, dialog }
}
