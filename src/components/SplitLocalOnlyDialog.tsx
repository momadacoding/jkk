import { useEffect, useRef, useState } from 'react'
import { useFocusTrap } from './useFocusTrap'
import { Reveal } from './Reveal'
import { toErrorMessage } from '../engine/errors'
import { LOCAL_ONLY_EDIT_WARNING, splitSelectionProblem } from '../engine/localOnlyEdit'

export interface SplitLocalOnlyRequest {
  /** commit 的短 hash 与标题，给人认是哪一条。 */
  label: string
  /** 这个 commit 改过的文件（后端 `svn_local_only_files`）。 */
  files: string[]
  /** 拆。抛错时框留在原地显示错误，可以就地重试。 */
  split: (paths: string[]) => Promise<void>
}

/**
 * 「拆出部分文件上服务器」（使用报告 9.2）：从 Local-Only commit 里勾出要上服务器的文件，拆成它前面的
 * 一个普通 commit，其余的留在 Local-Only commit 里。
 *
 * 勾的是「要上服务器的」，不是「要留下的」：报告里的情形是 66 个文件里只有一个要交付。
 */
export function SplitLocalOnlyDialog({
  request,
  onClose,
}: {
  request: SplitLocalOnlyRequest
  onClose: () => void
}) {
  const [chosen, setChosen] = useState<ReadonlySet<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const boxRef = useRef<HTMLDivElement>(null)
  const firstRef = useRef<HTMLInputElement>(null)
  useFocusTrap(boxRef, () => {
    if (!busy) onClose()
  })
  useEffect(() => {
    firstRef.current?.focus({ preventScroll: true })
  }, [])

  const problem = splitSelectionProblem(request.files.length, chosen.size)
  const toggle = (file: string) =>
    setChosen((prev) => {
      const next = new Set(prev)
      if (next.has(file)) next.delete(file)
      else next.add(file)
      return next
    })

  const run = async () => {
    if (problem || busy) return
    setBusy(true)
    setError(null)
    try {
      // 按列表顺序交，不按勾的先后：后端不在乎，但日志和测试读起来稳定
      await request.split(request.files.filter((f) => chosen.has(f)))
      onClose()
    } catch (err) {
      setError(toErrorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="dialog-overlay">
      <div ref={boxRef} className="dialog dialog-wide" role="dialog" aria-label="拆出部分文件上服务器">
        <div className="dialog-title">拆出部分文件上服务器</div>
        <div className="dialog-message">
          {request.label}：勾出要上服务器的文件。它们拆成前面的一个普通 commit，交付时一起提交；
          没勾的留在「仅本地」commit 里。
        </div>
        <div className="dlg-note">{LOCAL_ONLY_EDIT_WARNING}</div>
        <ul className="follow-items">
          {request.files.map((f, i) => (
            <li key={f}>
              <label className="dlg-check">
                <input
                  ref={i === 0 ? firstRef : undefined}
                  type="checkbox"
                  checked={chosen.has(f)}
                  disabled={busy}
                  onChange={() => toggle(f)}
                />
                <span className="follow-path">{f}</span>
              </label>
            </li>
          ))}
        </ul>
        {/* 一个都没勾时按钮已经灰着，不再多一行提示；只改了一个文件的 commit 一打开就说 */}
        {problem && (chosen.size > 0 || request.files.length < 2) && <div className="dlg-note">{problem}</div>}
        {error && <Reveal className="dlg-warning">{error}</Reveal>}
        <div className="dialog-actions">
          <button disabled={busy} onClick={onClose}>
            取消
          </button>
          <button className="primary" disabled={busy || problem !== null} onClick={() => void run()}>
            {busy ? '正在拆分…' : `拆出 ${chosen.size} 个文件`}
          </button>
        </div>
      </div>
    </div>
  )
}
