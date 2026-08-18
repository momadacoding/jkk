import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { toErrorMessage } from '../engine/errors'

/** 检出了待删分支的 worktree（主工作区除外）。 */
export interface WorktreeTarget {
  /** 目录名，界面上显示这个。 */
  name: string
  /** 完整路径，删除时传给后端。 */
  path: string
}

export interface DeleteInfo {
  /** 对应的远程分支简称，如 `origin/feat/x`；没有就不显示勾选框。 */
  upstream: string | null
  /** 上游所在的远程名，如 `origin`。 */
  remote: string | null
  /** 该分支在远程上的名字，如 `feat/x`。 */
  remoteBranch: string | null
  /** 只在这个分支上、不在 `comparedTo` 上的提交数。 */
  unmerged: number
  /**
   * 上面那个数是拿哪个 ref 比出来的，如 `main`。
   *
   * 必须显示出来：用 worktree 时主仓库的 HEAD 常年停在不相干的地方甚至是游离的，
   * 只说「不在当前分支上」等于什么都没说。
   */
  comparedTo: string
  /**
   * `unmerged` 不为 0，但内容其实已经全在当前分支上了——squash merge 的典型
   * 表现。为真时那几个提交丢不了东西，警告降级成中性提示。
   */
  contentMerged: boolean
  /** 检出了这个分支的 worktree；没有就不显示那个勾选框。 */
  worktree: WorktreeTarget | null
}

/** 确认删除时回传的远程分支坐标；不删远程就是 null。 */
export interface RemoteTarget {
  remote: string
  branch: string
}

/** 这一次删除都要动哪些东西。 */
export interface DeleteTargets {
  /** 非 null 时连远程分支一起删。 */
  remote: RemoteTarget | null
  /** 非 null 时先把这个 worktree 删掉，否则本地分支根本删不动。 */
  worktreePath: string | null
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

function FolderGlyph() {
  return (
    <svg className="dlg-glyph" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <path d="M1.8 12.8V3.7h4.1l1.4 1.8h6.9v7.3z" />
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
  /** 失败时应当抛出——弹框据此留在原地并把错误显示出来，好让用户重试。 */
  onConfirm: (targets: DeleteTargets) => Promise<void>
}

/**
 * 删除分支的确认框。
 *
 * 之前右键 Delete 是直接 `git branch -D`——没有任何确认，未合并的提交就那么
 * 没了。这个框把代价摆出来，顺带提供「同时删远程分支」和「同时删 worktree」。
 *
 * 后者不只是方便：分支被 worktree 检出时 `git branch -D` 直接拒绝，没有这个
 * 勾选框的话这类分支在界面上根本删不掉。
 */
export function DeleteBranchDialog({ repoPath, name, onCancel, onConfirm }: DeleteBranchDialogProps) {
  const [info, setInfo] = useState<DeleteInfo | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [alsoRemote, setAlsoRemote] = useState(false)
  /**
   * 是否连 worktree 一起删。有 worktree 时**默认勾上**。
   *
   * 这一项和「删远程」不同，它不是可选的附赠动作：git 拒绝删除被 worktree
   * 检出的分支，不勾就是必然失败。默认勾上，再用下面那行说明讲清为什么。
   */
  const [alsoWorktree, setAlsoWorktree] = useState(false)
  /** 删除正在进行中。勾了远程时这一步要走网络，实测能到四五秒。 */
  const [busy, setBusy] = useState(false)
  const confirmRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (!repoPath) return
    let cancelled = false
    invoke<DeleteInfo>('branch_delete_info', { path: repoPath, name })
      .then((data) => {
        if (cancelled) return
        setInfo(data)
        setAlsoWorktree(data.worktree !== null)
      })
      .catch((err) => { if (!cancelled) setError(toErrorMessage(err)) })
    return () => { cancelled = true }
  }, [repoPath, name])

  // 远程信息还没回来就先不给删：这时候勾选框还没出现，
  // 用户根本没机会决定要不要连远程一起删
  const ready = info !== null

  /**
   * 跑删除，跑完之前弹框留在原地。
   *
   * 删远程要走一次网络往返（实测到 GitHub 四五秒）。以前这里是发出去就把
   * 弹框关掉，于是界面上四五秒毫无动静，看着像卡死；现在把这段等待显示出来。
   * 失败也不关，错误就地显示，用户可以直接再点一次。
   */
  const confirm = async () => {
    if (!info || busy) return
    const remote = alsoRemote && info.remote && info.remoteBranch
      ? { remote: info.remote, branch: info.remoteBranch }
      : null
    const worktreePath = alsoWorktree && info.worktree ? info.worktree.path : null
    setBusy(true)
    setError(null)
    try {
      await onConfirm({ remote, worktreePath })
    } catch (err) {
      setError(toErrorMessage(err))
      setBusy(false)
    }
  }

  // 信息回来之后再聚焦确认按钮：焦点得先落进弹框里，回车/Esc 才有着落，
  // 而且这样 Tab 一下就能到勾选框
  useEffect(() => { if (ready) confirmRef.current?.focus() }, [ready])

  useEffect(() => {
    // 删除跑起来之后 Esc 不再关闭：git 已经在动远程了，关掉只会让用户以为取消了
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && !busy) onCancel() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onCancel, busy])

  return (
    <div className="dialog-overlay" onMouseDown={() => { if (!busy) onCancel() }}>
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
              disabled={busy}
              onChange={(e) => setAlsoRemote(e.target.checked)}
            />
            <span>Also delete remote branch</span>
            <span className="dlg-value"><RemoteGlyph />{info.upstream}</span>
          </label>
        )}

        {info?.worktree && (
          <>
            <label className="dlg-field dlg-check">
              <input
                type="checkbox"
                checked={alsoWorktree}
                disabled={busy}
                onChange={(e) => setAlsoWorktree(e.target.checked)}
              />
              <span>Also delete worktree</span>
              <span className="dlg-value"><FolderGlyph />{info.worktree.name}</span>
            </label>
            {/* 默认勾上是有原因的，说出来——否则用户会以为这只是个可选项，
                取消勾选后撞上 git 那句 "used by worktree at ..." 一头雾水 */}
            <div className="dlg-note under-check">
              {alsoWorktree
                ? '该分支正被这个 worktree 检出，git 不允许删除被检出的分支，所以这一项默认勾上。'
                : '不删掉这个 worktree 的话，git 会拒绝删除该分支。'}
            </div>
          </>
        )}

        {/* 同一个数字，两种读法。squash merge 之后提交对象全换了新 SHA，这个数
            必然大于 0，但内容早就进当前分支了——那种情况下喊「只能靠 reflog 找回」
            纯属误报，而且每合并一个分支就误报一次，喊到最后真出事也没人看了 */}
        {info !== null && info.unmerged > 0 && (
          info.contentMerged ? (
            <div className="dlg-note">
              该分支有 {info.unmerged} 个提交不在 {info.comparedTo} 上，但改动已经进去了
              （squash 或 rebase 合并后的正常现象），删掉不会丢东西。
            </div>
          ) : (
            <div className="dlg-warning">
              该分支有 {info.unmerged} 个提交不在 {info.comparedTo} 上，删除后只能靠 reflog 找回。
            </div>
          )
        )}

        {error && <div className="dlg-warning">{error}</div>}

        {/* 删 worktree 只是把目录搬进回收站，毫秒级；真正要等网络的只有远程
            那一步，单独报它，免得用户以为界面卡了 */}
        {busy && (
          <div className="dlg-progress">
            <span className="dlg-spinner" />
            {alsoRemote ? '正在联系远程…' : '正在删除…'}
          </div>
        )}

        <div className="dialog-actions">
          <button disabled={busy} onClick={onCancel}>Cancel</button>
          <button
            ref={confirmRef}
            className="primary danger"
            disabled={!ready || busy}
            onClick={() => void confirm()}
          >
            {busy ? 'Deleting…' : 'Delete'}
          </button>
        </div>
      </div>
    </div>
  )
}
