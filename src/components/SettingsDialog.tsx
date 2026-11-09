import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { captureShortcut, displayShortcut, type AppSettingsView } from '../engine/appSettings'
import { toErrorMessage } from '../engine/errors'
import { setShortcutRecording } from '@/store/appSettingsStore'
import { Reveal } from './Reveal'
import { useFocusTrap } from './useFocusTrap'

interface SettingsDialogProps {
  /** 读设置失败时为 `null`，框里只显示 `loadError`。 */
  initial: AppSettingsView | null
  loadError?: string | null
  /** 保存成功就关框；失败抛出，错误留在框里。 */
  onSave: (
    closeToTray: boolean,
    showShortcut: string | null,
    svnAutoFollow: boolean,
  ) => Promise<unknown>
  onClose: () => void
}

/**
 * 应用设置：关窗进托盘、全局快捷键、SVN 自动跟进。
 *
 * 快捷键靠按键录制而不是手填：手填要用户自己知道 `Backquote` 这种键名，
 * 录制直接取 `KeyboardEvent.code`，规则与后端校验共用一份（`engine/appSettings.ts`）。
 */
export function SettingsDialog({ initial, loadError = null, onSave, onClose }: SettingsDialogProps) {
  const [closeToTray, setCloseToTray] = useState(initial?.closeToTray ?? true)
  const [shortcutOn, setShortcutOn] = useState(initial ? initial.showShortcut !== null : true)
  const [shortcut, setShortcut] = useState(
    initial?.showShortcut ?? initial?.defaultShortcut ?? '',
  )
  const [autoFollow, setAutoFollow] = useState(initial?.svnAutoFollow ?? true)
  const [recording, setRecording] = useState(false)
  const [hint, setHint] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const boxRef = useRef<HTMLDivElement>(null)
  // 录制中的 Esc 由录制框自己吃掉（stopPropagation），到不了 hook 的冒泡监听
  useFocusTrap(boxRef, () => { if (!busy) onClose() })

  // 录制状态同步给后端。cleanup 里复位：录到一半直接关框（Esc、取消、保存）也不能留着标记，
  // 否则之后快捷键就再也不会隐藏窗口
  useEffect(() => {
    if (!recording) return
    void setShortcutRecording(true)
    return () => { void setShortcutRecording(false) }
  }, [recording])

  /**
   * 物理右 Alt（AltGr）是否按着。不用 `getModifierState('AltGraph')`：Windows 上的 Chromium
   * 在带 AltGr 的布局下，左 Ctrl + 左 Alt 也可能报 AltGraph，默认组合就录不进来了。
   */
  const altRightDown = useRef(false)

  const onRecordKey = (e: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (e.code === 'AltRight') altRightDown.current = true
    if (!recording) return
    e.preventDefault()
    e.stopPropagation()
    const plain = !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey
    if (e.key === 'Escape' && plain) {
      setRecording(false)
      setHint(null)
      return
    }
    const result = captureShortcut({
      code: e.code,
      key: e.key,
      ctrlKey: e.ctrlKey,
      altKey: e.altKey,
      shiftKey: e.shiftKey,
      metaKey: e.metaKey,
      altGraph: altRightDown.current,
    })
    if (result.kind === 'ok') {
      setShortcut(result.accelerator)
      setRecording(false)
      setHint(null)
    } else if (result.kind === 'invalid') {
      setHint(result.reason)
    }
  }

  const save = async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await onSave(closeToTray, shortcutOn ? shortcut : null, autoFollow)
      onClose()
    } catch (err) {
      setError(toErrorMessage(err))
      setBusy(false)
    }
  }

  // 已保存的快捷键没注册上：只在用户还没改动它时提示，改了就以这次保存的结果为准
  const staleError =
    initial?.shortcutError && shortcutOn && shortcut === initial.showShortcut
      ? initial.shortcutError
      : null

  return (
    <div className="dialog-overlay">
      <div
        ref={boxRef}
        className="dialog settings-dialog"
        role="dialog"
        aria-label="设置"
      >
        <div className="dialog-title">设置</div>
        {!initial ? (
          <div className="dlg-warning" role="alert">
            读取设置失败：{loadError ?? '未知错误'}
          </div>
        ) : (
          <>
            <div className="settings-section">
              <div className="settings-heading">关闭窗口</div>
              <label className="dlg-check">
                <input
                  type="checkbox"
                  checked={closeToTray && initial.trayAvailable}
                  disabled={!initial.trayAvailable}
                  onChange={(e) => setCloseToTray(e.target.checked)}
                />{' '}
                点关闭按钮时隐藏到托盘，后台继续运行
              </label>
              <div className="settings-note">
                {!initial.trayAvailable
                  ? '托盘图标没能创建，点关闭按钮只能直接退出。'
                  : closeToTray
                    ? '单击托盘图标回到主界面；要彻底退出，右键托盘图标选「退出」。'
                    : '点关闭按钮会直接退出应用。'}
              </div>
            </div>
            <div className="settings-section">
              <div className="settings-heading">全局快捷键</div>
              <label className="dlg-check">
                <input
                  type="checkbox"
                  checked={shortcutOn}
                  onChange={(e) => {
                    setShortcutOn(e.target.checked)
                    setRecording(false)
                    setHint(null)
                  }}
                />{' '}
                在任何程序里按快捷键呼出主界面
              </label>
              <div className="settings-row">
                <button
                  type="button"
                  className={recording ? 'shortcut-recorder recording' : 'shortcut-recorder'}
                  aria-label="快捷键"
                  disabled={!shortcutOn || busy}
                  onClick={() => {
                    setRecording(true)
                    setHint(null)
                  }}
                  onKeyDown={onRecordKey}
                  onKeyUp={(e) => {
                    if (e.code === 'AltRight') altRightDown.current = false
                  }}
                  onBlur={() => {
                    altRightDown.current = false
                    setRecording(false)
                  }}
                >
                  {recording ? '请按下组合键…' : displayShortcut(shortcut || null)}
                </button>
                <button
                  type="button"
                  className="settings-link"
                  disabled={!shortcutOn || busy || shortcut === initial.defaultShortcut}
                  onClick={() => {
                    setShortcut(initial.defaultShortcut)
                    setRecording(false)
                    setHint(null)
                  }}
                >
                  恢复默认（{displayShortcut(initial.defaultShortcut)}）
                </button>
              </div>
              <div className="settings-note">
                {hint ?? '点击上面的按钮后按下新的组合键，Esc 取消。必须带 Ctrl、Alt 或 Win。'}
              </div>
              {staleError && <div className="dlg-warning">{staleError}</div>}
            </div>
            <div className="settings-section">
              <div className="settings-heading">SVN 混合仓库</div>
              <label className="dlg-check">
                <input
                  type="checkbox"
                  checked={autoFollow}
                  onChange={(e) => setAutoFollow(e.target.checked)}
                />{' '}
                TortoiseSVN 提交或更新之后自动跟进
              </label>
              <div className="settings-note">
                {autoFollow
                  ? '只移动 git 侧的基线与分支，不改工作区里的文件。有冲突、连不上服务器、暂存区拿不准时' +
                    '不自动，横幅上会写明原因，点「跟进」照样可以完成。'
                  : '关掉后，SVN 侧有新内容时只挂横幅，点「跟进」才会跟上。'}
              </div>
            </div>
          </>
        )}
        {error && (
          <Reveal className="dlg-warning" role="alert">
            {error}
          </Reveal>
        )}
        <div className="dialog-actions">
          {initial && (
            <button
              className="primary"
              disabled={busy || (shortcutOn && !shortcut)}
              onClick={() => void save()}
            >
              保存
            </button>
          )}
          <button disabled={busy} onClick={onClose}>
            {initial ? '取消' : '关闭'}
          </button>
        </div>
      </div>
    </div>
  )
}

export default SettingsDialog
