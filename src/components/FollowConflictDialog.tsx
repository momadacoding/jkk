import { useCallback, useEffect, useRef, useState } from 'react'
import { useFocusTrap } from './useFocusTrap'
import { toErrorMessage } from '../engine/errors'
import {
  CONFLICT_PREVIEW_NOTE,
  DISK_FOLLOW_LABEL,
  type FollowConflictFile,
  type FollowConflictModel,
} from '../engine/svnFollow'
import { WarnGlyph } from './ConfirmDialog'
import { Reveal } from './Reveal'
import type { CompareAction } from './FollowReportDialog'

/** 冲突框上的两个出路。 */
export type FollowConflictChoice = 'disk' | 'manual'

export interface FollowConflictRequest {
  model: FollowConflictModel
  /** 这个文件的「对比」。返回 `null` 表示没有可用的对比工具：退回文本预览。 */
  compare: (file: FollowConflictFile) => CompareAction | null
  /**
   * 点了「用磁盘上的内容跟进」或「手动跟进」之后要跑的活。跑完关框；抛错则留在框里显示，可以再点。
   * 结局由调用方在框关掉之后处理。
   */
  run: (choice: FollowConflictChoice) => Promise<void>
}

interface Pending extends FollowConflictRequest {
  resolve: () => void
  /** 每个请求一个，做 `<Dialog>` 的 key，理由见 `useConfirm`。 */
  id: number
}

const BUSY: Record<FollowConflictChoice, string> = {
  disk: '正在跟进…',
  manual: '正在手动跟进…',
}

function FileRow({
  f,
  action,
  onCompare,
}: {
  f: FollowConflictFile
  action: CompareAction | null
  onCompare: (f: FollowConflictFile, a: CompareAction) => void
}) {
  return (
    <li className="conflict-file">
      <div className="follow-path">{f.path}</div>
      <div className="conflict-origin">{f.origin}</div>
      {f.diskLine && (
        <div className={f.disk === 'usable' ? 'conflict-disk ok' : 'conflict-disk bad'}>
          {f.diskLine}
        </div>
      )}
      {action ? (
        <div className="conflict-compare">
          <button
            type="button"
            className="follow-compare"
            aria-label={`用 ${action.label} 对比 ${f.path}`}
            onClick={() => onCompare(f, action)}
          >
            用 {action.label} 对比
          </button>
          <span className="conflict-sides">{f.sides}</span>
        </div>
      ) : (
        f.preview && (
          <>
            <div className="conflict-origin">{CONFLICT_PREVIEW_NOTE}</div>
            <pre className="conflict-preview">{f.preview}</pre>
          </>
        )
      )}
    </li>
  )
}

function Dialog({ request, onDone }: { request: Pending; onDone: () => void }) {
  const { model: m } = request
  const [busy, setBusy] = useState<FollowConflictChoice | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [compareError, setCompareError] = useState<string | null>(null)
  const primaryRef = useRef<HTMLButtonElement>(null)
  const boxRef = useRef<HTMLDivElement>(null)

  const finish = () => {
    request.resolve()
    onDone()
  }
  // 跑着的时候 Esc 不关：命令还在动 ref，关了框结局就没人接
  useFocusTrap(boxRef, () => {
    if (!busy) finish()
  })

  useEffect(() => {
    // 按钮贴在框底：不加 preventScroll 的话框一打开就滚到最底下，冲突文件看不到
    primaryRef.current?.focus({ preventScroll: true })
  }, [])

  const choose = (choice: FollowConflictChoice) => {
    setBusy(choice)
    setError(null)
    request.run(choice).then(finish, (err) => {
      setBusy(null)
      setError(toErrorMessage(err))
    })
  }
  const onCompare = (f: FollowConflictFile, a: CompareAction) => {
    setCompareError(null)
    a.open().catch((err) => setCompareError(`${f.path}：${toErrorMessage(err)}`))
  }

  return (
    <div className="dialog-overlay">
      <div
        ref={boxRef}
        className="dialog dialog-wide follow-dialog"
        role="dialog"
        aria-label={m.title}
      >
        <div className="dlg-head">
          <span className="dlg-icon">
            <WarnGlyph />
          </span>
          <div>
            <div className="dialog-title">{m.title}</div>
            <div className="dialog-message">{m.intro}</div>
          </div>
        </div>

        <ul className="follow-items conflict-files">
          {m.files.map((f) => (
            <FileRow key={f.path} f={f} action={request.compare(f)} onCompare={onCompare} />
          ))}
        </ul>
        {compareError && (
          <div className="dlg-warning follow-compare-error" aria-live="polite">
            {compareError}
          </div>
        )}

        <div className="conflict-action">{m.action}</div>
        {m.warning && <div className="dlg-warning">{m.warning}</div>}
        {m.notes.map((n, i) => (
          <div key={i} className="conflict-note">
            {n}
          </div>
        ))}
        {/* 错误在内容末尾：主按钮可能是在框的顶部点的，不滚过去就像什么都没发生 */}
        {error && (
          <Reveal className="dlg-warning" role="alert">
            {error}
          </Reveal>
        )}

        <div className="dialog-actions">
          <button onClick={finish} disabled={busy !== null}>
            Cancel
          </button>
          <button
            ref={m.diskReady ? undefined : primaryRef}
            className={m.diskReady ? undefined : 'primary'}
            onClick={() => choose('manual')}
            disabled={busy !== null}
          >
            {busy === 'manual' ? BUSY.manual : '手动跟进'}
          </button>
          {m.diskReady && (
            <button
              ref={primaryRef}
              className="primary"
              onClick={() => choose('disk')}
              disabled={busy !== null}
            >
              {busy === 'disk' ? BUSY.disk : DISK_FOLLOW_LABEL}
            </button>
          )}
        </div>
      </div>
    </div>
  )
}

/**
 * 跟进冲突框（设计 2026-11-25-follow-resolve-with-disk §5）。用法与 `useConfirm` 相同，关掉时 Promise 结束。
 */
export function useFollowConflictDialog() {
  const [pending, setPending] = useState<Pending | null>(null)
  const seq = useRef(0)

  const showFollowConflict = useCallback(
    (request: FollowConflictRequest) =>
      new Promise<void>((resolve) => setPending({ ...request, resolve, id: ++seq.current })),
    [],
  )

  // key 与「只关自己」的理由同 useConfirm
  const dialog = pending ? (
    <Dialog
      key={pending.id}
      request={pending}
      onDone={() => setPending((p) => (p?.id === pending.id ? null : p))}
    />
  ) : null

  return { showFollowConflict, dialog }
}
