import { useCallback, useEffect, useRef, useState } from 'react'

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
}

interface Pending extends ConfirmRequest {
  resolve: (ok: boolean) => void
}

function WarnGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" strokeWidth="1.6">
      <path d="M12 4 2.5 20h19z" />
      <path d="M12 10v5M12 17.5v.5" />
    </svg>
  )
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

  const finish = (ok: boolean) => {
    request.resolve(ok)
    onDone()
  }

  // 焦点必须落进弹框里，回车和 Esc 才有着落
  useEffect(() => { confirmRef.current?.focus() }, [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') finish(false) }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <div className="dialog-overlay" onMouseDown={() => finish(false)}>
      <div className="dialog dialog-wide" onMouseDown={(e) => e.stopPropagation()}>
        <div className="dlg-head">
          <span className={request.danger ? 'dlg-icon danger' : 'dlg-icon'}><WarnGlyph /></span>
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

        <div className="dialog-actions">
          <button onClick={() => finish(false)}>Cancel</button>
          <button
            ref={confirmRef}
            className={request.danger ? 'primary danger' : 'primary'}
            onClick={() => finish(true)}
          >
            {request.confirmLabel ?? 'OK'}
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

  const confirm = useCallback(
    (request: ConfirmRequest) =>
      new Promise<boolean>((resolve) => setPending({ ...request, resolve })),
    [],
  )

  const dialog = pending ? (
    <Dialog request={pending} onDone={() => setPending(null)} />
  ) : null

  return { confirm, dialog }
}
