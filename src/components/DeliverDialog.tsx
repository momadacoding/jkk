import { useCallback, useEffect, useRef, useState } from 'react'
import { toErrorMessage } from '../engine/errors'
import { canDeliver } from '../engine/svnSyncOut'
import { WarnGlyph } from './ConfirmDialog'
import type { DeliverRequest } from './svnSyncOutFlow'

interface Pending extends DeliverRequest {
  resolve: (ok: boolean) => void
  /** 每个请求一个，做 `<Dialog>` 的 key，理由见 `useConfirm`。 */
  id: number
}

function Dialog({ request, onDone }: { request: Pending; onDone: () => void }) {
  const m = request.model
  const [message, setMessage] = useState(request.initialMessage)
  const [ack, setAck] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const textRef = useRef<HTMLTextAreaElement>(null)
  const ready = canDeliver(message, ack, m) && !busy

  const finish = (ok: boolean) => {
    request.resolve(ok)
    onDone()
  }

  const accept = async () => {
    if (!ready) return
    setBusy(true)
    setError(null)
    try {
      await request.action(message)
      finish(true)
    } catch (err) {
      // 留在原地：提交信息还在框里，用户看完错误可以关掉去「更新」，回来时草稿还在
      setError(toErrorMessage(err))
      setBusy(false)
    }
  }

  // 焦点落进输入框：打开就能写提交信息
  useEffect(() => {
    textRef.current?.focus()
  }, [])

  useEffect(() => {
    // 交付跑起来之后 Esc 不再关闭：关掉只会让用户以为取消了，而 svn commit 可能已经发出
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !busy) finish(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [busy])

  return (
    <div className="dialog-overlay" onMouseDown={() => { if (!busy) finish(false) }}>
      <div
        className="dialog dialog-wide deliver-dialog"
        role="dialog"
        aria-label={m.title}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="dlg-head">
          {/* 提交不能撤回、直接影响全组：与删除类确认同一个红色图标 */}
          <span className="dlg-icon danger"><WarnGlyph /></span>
          <div>
            <div className="dialog-title">{m.title}</div>
            <div className="dialog-message">{m.summary}</div>
          </div>
        </div>

        <div className="deliver-files">
          {m.groups.map((g) => (
            <div key={g.label}>
              <div className="deliver-group">{g.label}</div>
              <ul>
                {g.files.map((f) => (
                  <li key={f}>{f}</li>
                ))}
              </ul>
            </div>
          ))}
        </div>

        <div className="dlg-note">{m.localOnlyNote}</div>
        {m.localOnly.length > 0 && (
          <ul className="deliver-local-only">
            {m.localOnly.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
        )}

        {m.outsideWarning && (
          <div className="dlg-warning">
            {m.outsideWarning}
            <label className="dlg-check deliver-ack">
              <input
                type="checkbox"
                checked={ack}
                disabled={busy}
                onChange={(e) => setAck(e.target.checked)}
              />
              我知道这些文件可能被合并，继续交付
            </label>
          </div>
        )}

        <label className="deliver-label" htmlFor="deliver-message">提交信息</label>
        <textarea
          id="deliver-message"
          className="deliver-message"
          ref={textRef}
          rows={4}
          value={message}
          disabled={busy}
          placeholder="Ctrl+Enter 交付"
          onChange={(e) => {
            setMessage(e.target.value)
            request.onMessageChange(e.target.value)
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
              e.preventDefault()
              void accept()
            }
          }}
        />

        {error && <div className="dlg-warning">{error}</div>}
        {busy && (
          <div className="dlg-progress">
            <span className="dlg-spinner" />
            {m.busyLabel}
          </div>
        )}

        <div className="dialog-actions">
          <button disabled={busy} onClick={() => finish(false)}>Cancel</button>
          <button className="primary danger" disabled={!ready} onClick={() => void accept()}>
            {busy ? '…' : m.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  )
}

/** 交付确认框。用法与 `useConfirm` 相同；要输入提交信息，所以单独一个组件。 */
export function useDeliverDialog() {
  const [pending, setPending] = useState<Pending | null>(null)
  const seq = useRef(0)

  const deliver = useCallback(
    (request: DeliverRequest) =>
      new Promise<boolean>((resolve) => setPending({ ...request, resolve, id: ++seq.current })),
    [],
  )

  // key 与「只关自己」的理由同 useConfirm（2026-09-28 E2E 场景 2、PR #49 Codex review #5）
  const dialog = pending ? (
    <Dialog
      key={pending.id}
      request={pending}
      onDone={() => setPending((p) => (p?.id === pending.id ? null : p))}
    />
  ) : null

  return { deliver, dialog }
}
