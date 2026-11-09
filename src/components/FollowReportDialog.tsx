import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useFocusTrap } from './useFocusTrap'
import { toErrorMessage } from '../engine/errors'
import {
  describeFollowReport,
  needsSvnUserPrompt,
  type FollowReport,
  type FollowReportSection,
  type ReportLevel,
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
    <div className="follow-user">
      <label htmlFor="follow-svn-user" className="follow-user-label">
        我的 SVN 用户名
      </label>
      <div className="follow-user-hint">
        用来判断「这次 SVN 提交是不是我的」。是我的，分支和暂存里的旧版本才能让位给 SVN 的版本；
        取不到时一律当成别人的提交，不会丢东西，只是要多确认几次。
      </div>
      <div className="follow-user-row">
        <input
          id="follow-svn-user"
          placeholder="提交 SVN 时用的用户名，例如 zhang.san"
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
      {saved && <div className="follow-user-saved">已保存，下次跟进时生效。</div>}
      {error && <div className="dlg-warning">{error}</div>}
    </div>
  )
}

/** info 节超过这么多条就先折起来：几千个跟进的文件不该把要处理的事挤出视野。 */
const INFO_OPEN_MAX = 8

const LEVEL_MARK: Record<ReportLevel, string> = { danger: '需处理', attention: '请确认', info: '' }

/** 只有一组时，一节默认列这么多条。上百条的危险清单全铺开，后面的节和页脚就得滚很久才看得到。 */
const PREVIEW_MAX = 10
/** 有好几组时每组先列这么多：每组的小标题都得露出来，按作者分组就是为了一眼看出谁改了什么。 */
const PREVIEW_PER_GROUP = 5

function preview(groups: FollowReportSection['groups']): FollowReportSection['groups'] {
  const per = groups.length === 1 ? PREVIEW_MAX : PREVIEW_PER_GROUP
  return groups.map((g) => ({ ...g, items: g.items.slice(0, per) }))
}

/** 真能铺开的条数：engine 截断后补的「…还有 N 个」不算。 */
const real = (groups: FollowReportSection['groups']) =>
  groups.reduce((n, g) => n + g.items.filter((i) => !i.more).length, 0)

/**
 * 报告里的一节。要留意的永远展开；只是告知的，条数多就折起来，标题和条数仍然看得到。
 *
 * 折叠由 state 管、折起来时正文不渲染：useFocusTrap 按选择器挑 Tab 的落点，藏在关着的
 * `<details>` 里的按钮也会被挑中，focus() 落不上，Tab 就卡在原地。`<summary>` 带 tabIndex
 * 才进得了那个选择器。
 */
function Section({ s }: { s: FollowReportSection }) {
  const [open, setOpen] = useState(s.level !== 'info' || s.count <= INFO_OPEN_MAX)
  const [all, setAll] = useState(false)
  const shown = all ? s.groups : preview(s.groups)
  const hidden = real(s.groups) - real(shown)
  return (
    <details className={`follow-section ${s.level}`} open={open}>
      <summary
        tabIndex={0}
        onClick={(e) => {
          // 原生的开合交给 state，否则 React 渲染的 open 与 DOM 对不上
          e.preventDefault()
          setOpen((o) => !o)
        }}
      >
        <span className="follow-section-title">{s.title}</span>
        {s.count > 0 && <span className="follow-count">{s.count}</span>}
        {LEVEL_MARK[s.level] && <span className="follow-mark">{LEVEL_MARK[s.level]}</span>}
      </summary>
      {open && (
        <>
          {s.note && <div className="follow-note">{s.note}</div>}
          {shown.map((g) => (
            <div key={g.label} className="follow-group">
              {g.label && <div className="follow-group-label">{g.label}</div>}
              <ul className="follow-items">
                {/* 同一路径可能在两个 commit 里泄漏，文案相同，不能拿它做 key */}
                {g.items.map((item, i) => (
                  <li key={i} className={item.more ? 'more' : undefined}>
                    <span className="follow-path">{item.text}</span>
                    {item.meta && <span className="follow-meta">{item.meta}</span>}
                  </li>
                ))}
              </ul>
            </div>
          ))}
          {(all || hidden > 0) && (
            <button className="follow-more" onClick={() => setAll((a) => !a)}>
              {all ? '收起' : `再显示 ${hidden} 个`}
            </button>
          )}
        </>
      )}
    </details>
  )
}

function Dialog({ request, onDone }: { request: Pending; onDone: () => void }) {
  const m = useMemo(() => describeFollowReport(request.report), [request.report])
  const [info, setInfo] = useState<SvnUserInfo | null>(null)
  const okRef = useRef<HTMLButtonElement>(null)
  const boxRef = useRef<HTMLDivElement>(null)

  const finish = () => {
    request.resolve()
    onDone()
  }
  useFocusTrap(boxRef, finish)

  useEffect(() => {
    // 按钮贴在框底：不加 preventScroll，框一打开就滚到最底下，摘要和要处理的事都看不到
    okRef.current?.focus({ preventScroll: true })
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

  const flagged = m.sections.filter((s) => s.level !== 'info')
  const done = m.sections.filter((s) => s.level === 'info')

  return (
    <div className="dialog-overlay">
      <div
        ref={boxRef}
        className="dialog dialog-wide follow-dialog"
        role="dialog"
        aria-label={m.title}
      >
        <div className="dlg-head">
          <span className={`dlg-icon ${m.level}`}>
            {m.level === 'ok' ? <CheckGlyph /> : <WarnGlyph />}
          </span>
          <div>
            <div className="dialog-title">{m.title}</div>
            <div className="dialog-message">{m.summary}</div>
            <div className="follow-safe">✓ {m.safe}</div>
          </div>
        </div>

        {flagged.length > 0 && (
          <div className="follow-zone">
            <div className="follow-zone-title">需要你留意</div>
            {flagged.map((s) => (
              <Section key={s.id} s={s} />
            ))}
          </div>
        )}

        {done.length > 0 && (
          <div className="follow-zone">
            <div className="follow-zone-title">这次跟进做了什么</div>
            {done.map((s) => (
              <Section key={s.id} s={s} />
            ))}
          </div>
        )}

        <div className="follow-footer">
          <div className="follow-basis">{m.basis}</div>
          {info && needsSvnUserPrompt(request.report, info) && (
            <SvnUserRow initial={info.configured ?? info.detected ?? ''} save={request.saveUser} />
          )}
        </div>

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
