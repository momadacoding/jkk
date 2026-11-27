import { useCallback, useEffect, useRef, useState } from 'react'
import { useFocusTrap } from './useFocusTrap'
import { toErrorMessage } from '../engine/errors'
import {
  ACTION_LABELS,
  BUSY_LABELS,
  describeConflictList,
  describeResolveWarning,
  needsConfirm,
  type ConfirmChoice,
  type ConflictAction,
  type ConflictRow,
  type ResolveChoice,
  type SvnConflict,
} from '../engine/svnConflicts'
import { WarnGlyph } from './ConfirmDialog'

/** 外部合并工具：按工具配置的规则选出来的那一个（`toolFor`），配了才有。 */
export interface ConflictMergeAction {
  label: string
  open: () => Promise<void>
}

export interface SvnConflictsRequest {
  /** 打开时已经查好的冲突（`offerSvnConflicts` 查过才决定弹不弹）。 */
  initial: SvnConflict[]
  /** 重读冲突。每次解决之后、点「刷新」时调。 */
  load: () => Promise<SvnConflict[]>
  resolve: (file: string, choice: ResolveChoice) => Promise<void>
  /** 这个文件的外部合并工具。`null`：没配可用的合并工具。 */
  merge: (c: SvnConflict) => ConflictMergeAction | null
}

interface Pending extends SvnConflictsRequest {
  resolve2: () => void
  id: number
}

/** 要先确认的操作（`needsConfirm`）点了之后在这一行里确认：都会改写文件，不能撤销。 */
type Confirming = { path: string; choice: ConfirmChoice }

function Row({
  row,
  conflict,
  merge,
  busy,
  anyBusy,
  confirming,
  error,
  onAction,
  onConfirm,
  onCancel,
}: {
  row: ConflictRow
  conflict: SvnConflict
  merge: ConflictMergeAction | null
  /** 这一行正在跑的操作（只管按钮上的字）。 */
  busy: ConflictAction | null
  /**
   * 框里任何一行在跑。按钮一律按它禁用：两行同时 `svn resolve` 会抢 wc.db 的写锁，先完成的那一行还会把
   * 「在跑」清掉，Esc 就能在另一行没结束时关框（独立 review）。
   */
  anyBusy: boolean
  confirming: ConfirmChoice | null
  error: string | null
  onAction: (a: ConflictAction) => void
  onConfirm: () => void
  onCancel: () => void
}) {
  // 没配合并工具时，「用外部工具合并」与「标记已解决」都不给：没有地方能把结果存出来
  const actions = row.actions.filter((a) => (a === 'merge' || a === 'merged' ? merge !== null : true))
  const label = (a: ConflictAction) =>
    busy === a ? BUSY_LABELS[a] : a === 'merge' && merge ? `用 ${merge.label} 合并` : ACTION_LABELS[a]
  return (
    <li className="conflict-file">
      <div className="follow-path">{row.path}</div>
      <div className="conflict-origin">{row.kind}</div>
      {actions.length > 0 && (
        <div className="conflict-compare">
          {actions.map((a) => (
            <button
              key={a}
              type="button"
              className="follow-compare"
              aria-label={`${row.path}：${a === 'merge' && merge ? `用 ${merge.label} 合并` : ACTION_LABELS[a]}`}
              onClick={() => onAction(a)}
              disabled={anyBusy || confirming !== null}
            >
              {label(a)}
            </button>
          ))}
        </div>
      )}
      {confirming && (
        <div className="dlg-warning svn-conflict-confirm" role="alert">
          <div>{describeResolveWarning(conflict, confirming)}</div>
          <div className="conflict-compare">
            <button type="button" className="follow-compare" onClick={onConfirm} disabled={anyBusy}>
              {busy ? BUSY_LABELS[confirming] : `确定${ACTION_LABELS[confirming]}`}
            </button>
            <button type="button" className="follow-compare" onClick={onCancel} disabled={anyBusy}>
              不改了
            </button>
          </div>
        </div>
      )}
      {/* 错误挂在点的那一行下面：列表可能很长，挂在框底会出在视野外 */}
      {error && (
        <div className="dlg-warning" role="alert">
          {error}
        </div>
      )}
    </li>
  )
}

function Dialog({ request, onDone }: { request: Pending; onDone: () => void }) {
  const [conflicts, setConflicts] = useState<SvnConflict[]>(request.initial)
  const [busy, setBusy] = useState<{ path: string; action: ConflictAction } | null>(null)
  const [confirming, setConfirming] = useState<Confirming | null>(null)
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [loadError, setLoadError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const primaryRef = useRef<HTMLButtonElement>(null)
  const boxRef = useRef<HTMLDivElement>(null)
  const m = describeConflictList(conflicts)

  const finish = () => {
    request.resolve2()
    onDone()
  }
  // 跑着的时候 Esc 不关：svn resolve 还在写 wc.db，关了框就没人报它的结局
  useFocusTrap(boxRef, () => {
    if (!busy) finish()
  })
  useEffect(() => {
    primaryRef.current?.focus({ preventScroll: true })
  }, [])

  const reload = async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setConflicts(await request.load())
    } catch (err) {
      setLoadError(toErrorMessage(err))
    } finally {
      setLoading(false)
    }
  }

  const setError = (path: string, msg: string | null) =>
    setErrors((e) => {
      const next = { ...e }
      if (msg === null) delete next[path]
      else next[path] = msg
      return next
    })

  const run = async (c: SvnConflict, action: ConflictAction, work: () => Promise<void>, reloadAfter: boolean) => {
    setBusy({ path: c.path, action })
    setError(c.path, null)
    try {
      await work()
      setConfirming(null)
      if (reloadAfter) await reload()
    } catch (err) {
      setError(c.path, toErrorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  const onAction = (c: SvnConflict, a: ConflictAction) => {
    if (needsConfirm(c, a)) {
      setError(c.path, null)
      setConfirming({ path: c.path, choice: a })
      return
    }
    if (a === 'merge') {
      const action = request.merge(c)
      if (action) void run(c, a, action.open, false)
      return
    }
    void run(c, a, () => request.resolve(c.path, 'merged'), true)
  }

  const byPath = (path: string) => conflicts.find((c) => c.path === path)!

  return (
    <div className="dialog-overlay">
      <div ref={boxRef} className="dialog dialog-wide follow-dialog" role="dialog" aria-label={m.title}>
        <div className="dlg-head">
          <span className="dlg-icon">
            <WarnGlyph />
          </span>
          <div>
            <div className="dialog-title">{m.title}</div>
            <div className="dialog-message">{m.intro}</div>
          </div>
        </div>
        {loadError && (
          <div className="dlg-warning" role="alert">
            {loadError}
          </div>
        )}

        {m.rows.length > 0 && (
          <ul className="follow-items conflict-files">
            {m.rows.map((row) => {
              const c = byPath(row.path)
              return (
                <Row
                  key={row.path}
                  row={row}
                  conflict={c}
                  merge={request.merge(c)}
                  busy={busy?.path === row.path ? busy.action : null}
                  anyBusy={busy !== null}
                  confirming={confirming?.path === row.path ? confirming.choice : null}
                  error={errors[row.path] ?? null}
                  onAction={(a) => onAction(c, a)}
                  onConfirm={() => {
                    if (!confirming) return
                    const choice = confirming.choice
                    void run(c, choice, () => request.resolve(c.path, choice), true)
                  }}
                  onCancel={() => setConfirming(null)}
                />
              )
            })}
          </ul>
        )}

        {m.refused.length > 0 && (
          <>
            <div className="conflict-action">{m.refusedNote}</div>
            <ul className="follow-items conflict-files">
              {m.refused.map((row) => (
                <li key={row.path} className="conflict-file">
                  <div className="follow-path">{row.path}</div>
                  <div className="conflict-origin">{row.kind}</div>
                </li>
              ))}
            </ul>
          </>
        )}

        {m.notes.map((n, i) => (
          <div key={i} className="conflict-note">
            {n}
          </div>
        ))}

        <div className="dialog-actions">
          <button onClick={() => void reload()} disabled={busy !== null || loading}>
            {loading ? '正在刷新…' : '刷新'}
          </button>
          <button ref={primaryRef} className="primary" onClick={finish} disabled={busy !== null}>
            关闭
          </button>
        </div>
      </div>
    </div>
  )
}

/**
 * SVN 冲突列表（设计 2026-11-28-update-conflict-resolve-in-jkk §3.2）。关掉时 Promise 结束。
 *
 * 已经开着时再弹一次（例如又点了一次「更新」被拒）：换成新的请求，列表按新查到的显示。
 */
export function useSvnConflictsDialog() {
  const [pending, setPending] = useState<Pending | null>(null)
  const seq = useRef(0)
  // 当前开着的那一次。放 ref 而不是在 setPending 的 updater 里读：updater 可能被 React 调两次，副作用不能放里面
  const current = useRef<Pending | null>(null)

  const showSvnConflicts = useCallback(
    (request: SvnConflictsRequest) =>
      new Promise<void>((resolve2) => {
        // 被换掉的那一次也算关了：等它的调用方不能永远挂着
        current.current?.resolve2()
        const next = { ...request, resolve2, id: ++seq.current }
        current.current = next
        setPending(next)
      }),
    [],
  )

  const dialog = pending ? (
    <Dialog
      key={pending.id}
      request={pending}
      onDone={() => {
        if (current.current?.id === pending.id) current.current = null
        setPending((p) => (p?.id === pending.id ? null : p))
      }}
    />
  ) : null

  return { showSvnConflicts, dialog }
}
