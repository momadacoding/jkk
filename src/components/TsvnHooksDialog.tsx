import { useEffect, useRef, useState } from 'react'
import {
  entriesToWrite,
  healthLabel,
  isDevBuildExe,
  type HookEntry,
  type TsvnHooksStatus,
} from '../engine/tsvnHooks'
import { useFocusTrap } from './useFocusTrap'
import { Reveal } from './Reveal'

interface TsvnHooksDialogProps {
  status: TsvnHooksStatus
  /** 写注册表进行中：按钮禁用，连点不会写第二次（第二次的 CAS 会报「被别的程序改过」）。 */
  busy?: boolean
  /** 上一次注册 / 撤销失败的原因。顶部红条被遮罩盖着，要显示在框内。 */
  error?: string | null
  onRegister: (blockLeaks: boolean) => void
  onUnregister: () => void
  onClose: () => void
}

function EntryList({ entries }: { entries: HookEntry[] }) {
  return (
    <ul>
      {entries.map((e, i) => (
        <li key={i}>
          <div>
            <strong>{e.kind}</strong>
          </div>
          <div>{e.wcPath}</div>
          <code>{e.command}</code>
        </li>
      ))}
    </ul>
  )
}

/**
 * 注册 / 撤销 TortoiseSVN 客户端钩子（分析 §6.13.5）。注册前列出将写入的每一条，用户确认后
 * 才写 TortoiseSVN 的设置；同类钩子已被别人占着时不写。
 */
export function TsvnHooksDialog({
  status,
  busy = false,
  error = null,
  onRegister,
  onUnregister,
  onClose,
}: TsvnHooksDialogProps) {
  const [blockLeaks, setBlockLeaks] = useState(
    status.health.state === 'healthy' ? status.health.blockLeaks : true,
  )
  const boxRef = useRef<HTMLDivElement>(null)
  useFocusTrap(boxRef)
  // 遮罩不关框之后，Esc 是「关闭」按钮之外唯一的快捷出口。挂 window：打开时焦点不在框里。
  // 执行中也照关，与「关闭」按钮一致（它不随 busy 禁用）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  const registered = status.health.state !== 'notRegistered'
  const toWrite = entriesToWrite(status.planned, blockLeaks)
  const conflicts = entriesToWrite(status.conflicts, blockLeaks)
  const blocked = conflicts.length > 0 || status.planned.length === 0

  return (
    <div className="dialog-overlay">
      <div
        ref={boxRef}
        className="dialog dialog-wide"
        role="dialog"
        aria-label="TortoiseSVN 钩子"
      >
        <div className="dialog-title">TortoiseSVN 钩子</div>
        {!status.supported ? (
          <div className="dialog-message">TortoiseSVN 钩子只在 Windows 上可用。</div>
        ) : (
          <>
            <div className="dialog-message">
              当前状态：{healthLabel(status.health)}。注册后，TortoiseSVN 提交和更新时会通知
              jkk，跟进时按真实发生的提交判定暂存是不是中间版本，不再靠猜。
            </div>
            <div className="dialog-message">
              注册前请先关掉 TortoiseSVN 的设置窗口：它开着时点确定，可能把这里写入的钩子盖掉。
            </div>
            {isDevBuildExe(status.exe) && (
              <div className="dlg-warning">
                这是开发构建（{status.exe}）。删掉这个目录之后，TortoiseSVN 提交会报钩子错误，请先撤销注册。
              </div>
            )}
            <label>
              <input
                type="checkbox"
                checked={blockLeaks}
                onChange={(e) => setBlockLeaks(e.target.checked)}
              />
              提交前拦截本地补丁
            </label>
            {status.planned.length === 0 ? (
              <div className="dlg-warning">没有找到要挂钩子的工作副本：请先检查白名单。</div>
            ) : (
              <div className="deliver-files">
                <div className="deliver-group">将写入 TortoiseSVN 设置的钩子</div>
                <EntryList entries={toWrite} />
              </div>
            )}
            {conflicts.length > 0 && (
              <div className="deliver-files">
                <div className="dlg-warning">
                  这些路径上已经有同类型的钩子，jkk 不覆盖。请先在 TortoiseSVN 设置 → Hook Scripts
                  里处理：
                </div>
                <EntryList entries={conflicts} />
              </div>
            )}
          </>
        )}
        {error && (
          <Reveal className="dlg-warning" role="alert">
            {error}
          </Reveal>
        )}
        <div className="dialog-actions">
          {status.supported && registered && (
            <button disabled={busy} onClick={onUnregister}>
              撤销注册
            </button>
          )}
          {status.supported && (
            <button
              className="primary"
              disabled={blocked || busy}
              onClick={() => onRegister(blockLeaks)}
            >
              {registered ? '重新注册' : '注册'}
            </button>
          )}
          <button onClick={onClose}>关闭</button>
        </div>
      </div>
    </div>
  )
}

export default TsvnHooksDialog
