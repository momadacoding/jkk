import { useEffect, useRef, useState } from 'react'
import type { Worktree } from '../engine/types'
import { toErrorCode, toErrorMessage } from '../engine/errors'

function TrashGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" strokeWidth="1.6">
      <path d="M4 7h16M10 4h4M9 7v12M15 7v12M6 7l1 13h10l1-13" />
    </svg>
  )
}

function FolderGlyph() {
  return (
    <svg className="dlg-glyph" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <path d="M1.8 12.8V3.7h4.1l1.4 1.8h6.9v7.3z" />
    </svg>
  )
}

function BranchGlyph() {
  return (
    <svg className="dlg-glyph" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <circle cx="4.5" cy="3.5" r="1.7" />
      <circle cx="4.5" cy="12.5" r="1.7" />
      <circle cx="11.5" cy="6.5" r="1.7" />
      <path d="M4.5 5.2v5.6M11.5 8.2c0 2.2-2.6 2.4-4.4 3" />
    </svg>
  )
}

interface DeleteWorktreeDialogProps {
  worktree: Worktree
  onCancel: () => void
  /** 失败时应当抛出——弹框据此留在原地并把错误显示出来，好让用户重试。 */
  onConfirm: (force: boolean) => Promise<void>
}

/**
 * 删除 worktree 的确认框。
 *
 * 之前侧边栏那个 ✕ 是直接删的，没有任何确认——一个可能装着未提交代码的目录
 * 一键就没了。
 *
 * 「有未提交改动」这一层不预先查，而是**先按不带 force 删一次**，撞上后端的
 * `worktree_dirty` 再把按钮升级成「仍要删除」。这样省掉一个只为弹框服务的
 * 查询命令，而且判据和真正执行删除的是同一段代码，不会出现「弹框说干净、
 * 删的时候又说脏」这种自相矛盾。
 */
export function DeleteWorktreeDialog({ worktree, onCancel, onConfirm }: DeleteWorktreeDialogProps) {
  const [error, setError] = useState<string | null>(null)
  /** 后端说这个 worktree 是脏的，再点一次就带 force 删。 */
  const [needsForce, setNeedsForce] = useState(false)
  const [busy, setBusy] = useState(false)
  const confirmRef = useRef<HTMLButtonElement>(null)

  useEffect(() => { confirmRef.current?.focus() }, [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && !busy) onCancel() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onCancel, busy])

  const confirm = async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await onConfirm(needsForce)
    } catch (err) {
      const code = toErrorCode(err)
      // 脏和坏都是「用户确认后可以继续」的，其余（锁住、目录被占用）再点也没用
      if (code === 'worktree_dirty' || code === 'worktree_broken') setNeedsForce(true)
      setError(toErrorMessage(err))
      setBusy(false)
    }
  }

  return (
    <div className="dialog-overlay" onMouseDown={() => { if (!busy) onCancel() }}>
      <div className="dialog dialog-wide" onMouseDown={(e) => e.stopPropagation()}>
        <div className="dlg-head">
          <span className="dlg-icon danger"><TrashGlyph /></span>
          <div>
            <div className="dialog-title">Delete Worktree</div>
            <div className="dialog-message">Move this working tree out of the repository</div>
          </div>
        </div>

        <div className="dlg-field">
          <span className="dlg-field-label">Worktree:</span>
          <span className="dlg-value"><FolderGlyph />{worktree.name}</span>
        </div>

        {worktree.branch && (
          <div className="dlg-field">
            <span className="dlg-field-label">Branch:</span>
            <span className="dlg-value"><BranchGlyph />{worktree.branch}</span>
          </div>
        )}

        <div className="dlg-path" title={worktree.path}>{worktree.path}</div>

        {/* 说清楚代价的边界：目录能撤销，分支根本不动。不写的话用户会以为
            删 worktree 会把分支上的提交一起带走 */}
        <div className="dlg-note">
          目录会被移到仓库内的回收站，删完可以撤销。分支和它上面的提交不受影响。
        </div>

        {error && <div className="dlg-warning">{error}</div>}

        {busy && (
          <div className="dlg-progress">
            <span className="dlg-spinner" />
            正在删除…
          </div>
        )}

        <div className="dialog-actions">
          <button disabled={busy} onClick={onCancel}>Cancel</button>
          <button
            ref={confirmRef}
            className="primary danger"
            disabled={busy}
            onClick={() => void confirm()}
          >
            {busy ? 'Deleting…' : needsForce ? '仍要删除' : 'Delete'}
          </button>
        </div>
      </div>
    </div>
  )
}
