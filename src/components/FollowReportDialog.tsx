import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useFocusTrap } from './useFocusTrap'
import { toErrorMessage } from '../engine/errors'
import {
  describeFollowReport,
  needsSvnUserPrompt,
  type FollowReport,
  type SvnUserInfo,
} from '../engine/svnFollow'
import { CheckGlyph, WarnGlyph } from './ConfirmDialog'

export interface FollowReportRequest {
  report: FollowReport
  /** 读「我的 SVN 用户名」。读不出来按取不到处理：报告本身不依赖它。 */
  readUser: () => Promise<SvnUserInfo>
  saveUser: (user: string) => Promise<void>
}

interface Pending extends FollowReportRequest {
  resolve: () => void
  /** 每个请求一个，做 `<Dialog>` 的 key，理由见 `useConfirm`。 */
  id: number
}

const NOBODY: SvnUserInfo = { configured: null, detected: null }

/** 「我的 SVN 用户名」一行（日志 D4）。跟进不依赖它，所以存不存都不挡「知道了」。 */
function SvnUserRow({
  initial,
  save,
}: {
  initial: string
  save: (user: string) => Promise<void>
}) {
  const [user, setUser] = useState(initial)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async () => {
    setSaved(false)
    setError(null)
    try {
      await save(user)
      setSaved(true)
    } catch (err) {
      setError(toErrorMessage(err))
    }
  }

  return (
    <>
      <div className="dlg-note">
        <label htmlFor="follow-svn-user">我的 SVN 用户名</label>
        ：用来判断「这次 SVN 提交是不是我的」。是我的，分支和暂存里的旧版本才能让位给 SVN 的版本；
        取不到时一律当成别人的提交，不会丢东西，只是要多确认几次。
      </div>
      <div className="follow-user">
        <input
          id="follow-svn-user"
          value={user}
          onChange={(e) => {
            setUser(e.target.value)
            setSaved(false)
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              void submit()
            }
          }}
        />
        <button onClick={() => void submit()}>保存</button>
      </div>
      {saved && <div className="dlg-note">已保存，下次跟进时生效。</div>}
      {error && <div className="dlg-warning">{error}</div>}
    </>
  )
}

function Dialog({ request, onDone }: { request: Pending; onDone: () => void }) {
  const m = useMemo(() => describeFollowReport(request.report), [request.report])
  const [info, setInfo] = useState<SvnUserInfo | null>(null)
  const okRef = useRef<HTMLButtonElement>(null)
  const boxRef = useRef<HTMLDivElement>(null)
  useFocusTrap(boxRef)

  const finish = () => {
    request.resolve()
    onDone()
  }

  useEffect(() => {
    okRef.current?.focus()
  }, [])

  useEffect(() => {
    let alive = true
    void request
      .readUser()
      .catch(() => NOBODY)
      .then((i) => {
        if (alive) setInfo(i)
      })
    return () => {
      alive = false
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') finish()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <div className="dialog-overlay" onMouseDown={finish}>
      <div
        ref={boxRef}
        className="dialog dialog-wide deliver-dialog"
        role="dialog"
        aria-label={m.title}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="dlg-head">
          <span className={`dlg-icon ${m.tone}`}>
            {m.tone === 'ok' ? <CheckGlyph /> : <WarnGlyph />}
          </span>
          <div>
            <div className="dialog-title">{m.title}</div>
            <div className="dialog-message">{m.summary}</div>
          </div>
        </div>

        {m.warning && <div className="dlg-warning">{m.warning}</div>}
        <div className="dlg-note">{m.basis}</div>

        {m.sections.length > 0 && (
          <div className="deliver-files follow-report">
            {m.sections.map((s) => (
              <div key={s.id}>
                <div className="deliver-group">{s.title}</div>
                {s.note && <div className={s.warn ? 'dlg-warning' : 'dlg-note'}>{s.note}</div>}
                {s.groups.map((g) => (
                  <div key={g.label}>
                    {g.label && <div className="deliver-group">{g.label}</div>}
                    <ul>
                      {/* 同一路径可能在两个 commit 里泄漏，文案相同，不能拿它做 key */}
                      {g.items.map((item, i) => (
                        <li key={i}>{item}</li>
                      ))}
                    </ul>
                  </div>
                ))}
              </div>
            ))}
          </div>
        )}

        {info && needsSvnUserPrompt(request.report, info) && (
          <SvnUserRow initial={info.configured ?? info.detected ?? ''} save={request.saveUser} />
        )}

        <div className="dialog-actions">
          <button ref={okRef} className="primary" onClick={finish}>
            知道了
          </button>
        </div>
      </div>
    </div>
  )
}

/** 跟进报告（分析 §6.10、§6.12）。用法与 `useConfirm` 相同，关掉时 Promise 结束。 */
export function useFollowReportDialog() {
  const [pending, setPending] = useState<Pending | null>(null)
  const seq = useRef(0)

  const showFollowReport = useCallback(
    (request: FollowReportRequest) =>
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

  return { showFollowReport, dialog }
}
