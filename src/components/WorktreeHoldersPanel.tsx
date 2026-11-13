import { useCallback, useEffect, useRef, useState } from 'react'
import { listWorktreeHolders, terminateWorktreeHolder } from '@/store/repoStore'
import { toErrorMessage } from '../engine/errors'
import { holderReason, notKillableReason, type WorktreeHolder } from '../engine/worktreeHolders'

/** 「确认结束」等多久没点就复原。走开一阵回来，一下点击不该直接结束进程。 */
export const ARM_TIMEOUT_MS = 5000

interface WorktreeHoldersPanelProps {
  /** 主仓库路径，后端据此核对 `worktreePath` 是登记在册的 worktree。 */
  repoPath: string
  worktreePath: string
  /** 删除正在进行中：这时不该再去结束进程。 */
  disabled: boolean
  /** 开始 / 结束「正在结束某个进程」时调。父组件据此禁用 Delete，免得和结束并发。 */
  onEndingChange?: (ending: boolean) => void
}

/**
 * 删 worktree 撞上 `worktree_busy` 时挂在错误下面：列出占着它的进程，逐个给「结束进程」。
 *
 * 以前只有一句「查不出是哪个进程」，用户得自己去任务管理器里猜。真实案例是一个父进程
 * 早已退出的孤儿 node.exe 把 worktree 当工作目录挂了 16 天。
 *
 * **结束要点两下。** 第一下只把按钮变成「确认结束」：结束进程会丢掉那个进程里没保存的
 * 东西，误点一下就动手太重；再弹一层确认框又打断删除弹框本身。「确认结束」几秒不点就复原。
 *
 * 结束完不自动重试删除：删除可能还有别的原因失败（还有别的进程、目录被锁），
 * 让用户自己再点一次 Delete，结果在弹框里看得清清楚楚。
 */
export function WorktreeHoldersPanel({ repoPath, worktreePath, disabled, onEndingChange }: WorktreeHoldersPanelProps) {
  /** null = 还在查。 */
  const [holders, setHolders] = useState<WorktreeHolder[] | null>(null)
  /** 结束失败的原因。和查询失败分开存，后一次重查失败不能把它盖掉。 */
  const [endError, setEndError] = useState<string | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  /** 点过一下、等确认的那个进程。 */
  const [armed, setArmed] = useState<number | null>(null)
  /** 正在结束的那个进程。 */
  const [ending, setEnding] = useState<number | null>(null)
  /** 这个面板里结束过进程：列表空了时据此提示「可以重试了」。 */
  const [endedAny, setEndedAny] = useState(false)
  /** 卸载后不再写 state（弹框可能在查询途中被关掉）。 */
  const alive = useRef(true)
  // 挂载时也要设回 true：dev 下 StrictMode 会挂载→卸载→再挂载，只在卸载时写 false 的话，
  // 第二次挂载之后它一直是 false，查回来的结果全被丢掉（tauri dev 实测，面板卡在「正在查」）
  useEffect(() => {
    alive.current = true
    return () => { alive.current = false }
  }, [])

  const onEndingRef = useRef(onEndingChange)
  onEndingRef.current = onEndingChange
  useEffect(() => { onEndingRef.current?.(ending !== null) }, [ending])
  // 卸载时还在结束中（弹框被关了），告诉父组件别一直禁着
  useEffect(() => () => { onEndingRef.current?.(false) }, [])

  useEffect(() => {
    if (armed === null) return
    const t = setTimeout(() => setArmed(null), ARM_TIMEOUT_MS)
    return () => clearTimeout(t)
  }, [armed])

  const load = useCallback(async () => {
    try {
      const list = await listWorktreeHolders(repoPath, worktreePath)
      if (!alive.current) return
      setHolders(list)
      setLoadError(null)
    } catch (err) {
      if (!alive.current) return
      setHolders((prev) => prev ?? [])
      setLoadError(toErrorMessage(err))
    }
  }, [repoPath, worktreePath])

  useEffect(() => { void load() }, [load])

  const end = async (holder: WorktreeHolder) => {
    if (armed !== holder.pid) {
      setArmed(holder.pid)
      return
    }
    setArmed(null)
    setEnding(holder.pid)
    setEndError(null)
    try {
      await terminateWorktreeHolder(repoPath, worktreePath, holder)
      if (alive.current) setEndedAny(true)
    } catch (err) {
      if (alive.current) setEndError(toErrorMessage(err))
    }
    // 成功失败都重查：结束一个进程可能连带它的子进程退出，失败则多半是它已经自己没了
    await load()
    if (alive.current) setEnding(null)
  }

  if (holders === null) {
    return (
      <div className="dlg-progress">
        <span className="dlg-spinner" />
        正在查是哪些进程占着它…
      </div>
    )
  }

  return (
    <>
      {holders.length > 0 && (
        <div className="dlg-holders">
          <div className="dlg-holders-title">占着这个 worktree 的进程：</div>
          {holders.map((h) => (
            <div key={h.pid} className="dlg-holder">
              <div className="dlg-holder-text">
                <span className="dlg-holder-name">{h.name}</span>
                <span className="dlg-holder-pid">PID {h.pid}</span>
                <div className="dlg-holder-reason">
                  {holderReason(h, worktreePath)}
                  {!h.killable && <>（{notKillableReason(h)}）</>}
                </div>
                {/* 只是旁证的那一档：把命令行摆出来，用户看得见依据才好判断要不要结束 */}
                {h.commandLine && <div className="dlg-holder-cmdline">{h.commandLine}</div>}
              </div>
              {h.killable && (
                <button
                  className={armed === h.pid ? 'danger' : undefined}
                  disabled={disabled || ending !== null}
                  onClick={() => void end(h)}
                >
                  {ending === h.pid ? '正在结束…' : armed === h.pid ? '确认结束' : '结束进程'}
                </button>
              )}
            </div>
          ))}
        </div>
      )}
      {holders.length === 0 && !loadError && (
        <div className="dlg-note">
          {endedAny
            ? '占着它的进程都结束了，再点一次删除试试。'
            : '没查到占着它的进程。多半是以管理员身份运行的程序，本应用读不到它的信息，请到任务管理器里找。'}
        </div>
      )}
      {endError && <div className="dlg-warning">{endError}</div>}
      {loadError && <div className="dlg-warning">查占用进程失败：{loadError}</div>}
    </>
  )
}
