import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { toErrorMessage } from '../engine/errors'

export interface DeleteInfo {
  /** 对应的远程分支简称，如 `origin/feat/x`；没有就不显示勾选框。 */
  upstream: string | null
  /** 上游所在的远程名，如 `origin`。 */
  remote: string | null
  /** 该分支在远程上的名字，如 `feat/x`。 */
  remoteBranch: string | null
  /** 只在这个分支上、不在当前 HEAD 上的提交数。 */
  unmerged: number
}

/** 确认删除时回传的远程分支坐标；不删远程就是 null。 */
export interface RemoteTarget {
  remote: string
  branch: string
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

function RemoteGlyph() {
  return (
    <svg className="dlg-glyph" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <path d="M4.5 12.5a3 3 0 0 1-.3-6 4 4 0 0 1 7.7-.6 2.8 2.8 0 0 1-.4 6.6z" />
    </svg>
  )
}

function TrashGlyph() {
  return (
    <svg viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" strokeWidth="1.6">
      <path d="M4 7h16M10 4h4M9 7v12M15 7v12M6 7l1 13h10l1-13" />
    </svg>
  )
}

interface DeleteBranchDialogProps {
  repoPath: string | null
  /** 要删除的本地分支名。 */
  name: string
  onCancel: () => void
  onConfirm: (remote: RemoteTarget | null) => void
}

/**
 * 删除分支的确认框。
 *
 * 之前右键 Delete 是直接 `git branch -D`——没有任何确认，未合并的提交就那么
 * 没了。这个框把代价摆出来，顺带提供「同时删远程分支」。
 */
export function DeleteBranchDialog({ repoPath, name, onCancel, onConfirm }: DeleteBranchDialogProps) {
  const [info, setInfo] = useState<DeleteInfo | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [alsoRemote, setAlsoRemote] = useState(false)
  const confirmRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (!repoPath) return
    let cancelled = false
    invoke<DeleteInfo>('branch_delete_info', { path: repoPath, name })
      .then((data) => { if (!cancelled) setInfo(data) })
      .catch((err) => { if (!cancelled) setError(toErrorMessage(err)) })
    return () => { cancelled = true }
  }, [repoPath, name])

  // 远程信息还没回来就先不给删：这时候勾选框还没出现，
  // 用户根本没机会决定要不要连远程一起删
  const ready = info !== null

  const confirm = () => {
    if (!info) return
    const target = alsoRemote && info.remote && info.remoteBranch
      ? { remote: info.remote, branch: info.remoteBranch }
      : null
    onConfirm(target)
  }

  // 信息回来之后再聚焦确认按钮：焦点得先落进弹框里，回车/Esc 才有着落，
  // 而且这样 Tab 一下就能到勾选框
  useEffect(() => { if (ready) confirmRef.current?.focus() }, [ready])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onCancel() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onCancel])

  return (
    <div className="dialog-overlay" onMouseDown={onCancel}>
      <div className="dialog dialog-wide" onMouseDown={(e) => e.stopPropagation()}>
        <div className="dlg-head">
          <span className="dlg-icon danger"><TrashGlyph /></span>
          <div>
            <div className="dialog-title">Delete Branch</div>
            <div className="dialog-message">Delete local branch from your repository</div>
          </div>
        </div>

        <div className="dlg-field">
          <span className="dlg-field-label">Branch:</span>
          <span className="dlg-value"><BranchGlyph />{name}</span>
        </div>

        {info?.upstream && (
          <label className="dlg-field dlg-check">
            <input
              type="checkbox"
              checked={alsoRemote}
              onChange={(e) => setAlsoRemote(e.target.checked)}
            />
            <span>Also delete remote branch</span>
            <span className="dlg-value"><RemoteGlyph />{info.upstream}</span>
          </label>
        )}

        {info !== null && info.unmerged > 0 && (
          <div className="dlg-warning">
            该分支有 {info.unmerged} 个提交不在当前分支上，删除后只能靠 reflog 找回。
          </div>
        )}

        {error && <div className="dlg-warning">{error}</div>}

        <div className="dialog-actions">
          <button onClick={onCancel}>Cancel</button>
          <button ref={confirmRef} className="primary danger" disabled={!ready} onClick={confirm}>
            Delete
          </button>
        </div>
      </div>
    </div>
  )
}
