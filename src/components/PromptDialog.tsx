import { useCallback, useEffect, useRef, useState } from 'react'
import { useFocusTrap } from './useFocusTrap'

export interface PromptRequest {
  /** 标题栏文字。 */
  title: string
  /** 输入框上方的说明。 */
  message: string
  /** 预填内容；改名这类场景会整段选中，方便直接覆写。 */
  defaultValue?: string
  /** 确认按钮文字，默认 OK。 */
  confirmLabel?: string
}

interface Pending extends PromptRequest {
  resolve: (value: string | null) => void
}

function Dialog({ request, onDone }: { request: Pending; onDone: () => void }) {
  const [value, setValue] = useState(request.defaultValue ?? '')
  const inputRef = useRef<HTMLInputElement>(null)
  const boxRef = useRef<HTMLDivElement>(null)
  useFocusTrap(boxRef)

  // 弹出来就聚焦并整段选中，对齐 window.prompt 的手感：直接打字就是覆写
  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])

  const finish = (result: string | null) => {
    request.resolve(result)
    onDone()
  }

  const trimmed = value.trim()

  return (
    <div className="dialog-overlay" onMouseDown={() => finish(null)}>
      {/* 点遮罩关闭，但点框内不能穿透过去 */}
      <div ref={boxRef} className="dialog" onMouseDown={(e) => e.stopPropagation()}>
        <div className="dialog-title">{request.title}</div>
        <label className="dialog-message" htmlFor="prompt-input">{request.message}</label>
        <input
          id="prompt-input"
          ref={inputRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && trimmed) finish(trimmed)
            if (e.key === 'Escape') finish(null)
          }}
        />
        <div className="dialog-actions">
          <button onClick={() => finish(null)}>Cancel</button>
          <button className="primary" disabled={!trimmed} onClick={() => finish(trimmed)}>
            {request.confirmLabel ?? 'OK'}
          </button>
        </div>
      </div>
    </div>
  )
}

/**
 * 应用内的文本输入弹框，替代 `window.prompt`。
 *
 * 不用 `window.prompt` 有两个原因：它在 webview 里会显示成
 * 「localhost:5174 says」这种带地址的原生框；而且它**同步阻塞主线程**——
 * 在右键菜单里调用时，React 来不及把菜单撤掉，菜单会一直留在弹框后面。
 *
 * Tauri 的 dialog 插件只有 confirm/message，没有文本输入，所以这里自己做。
 *
 * 用法：把 `dialog` 渲染出来，`prompt()` 返回用户输入（取消为 null）。
 */
export function usePrompt() {
  const [pending, setPending] = useState<Pending | null>(null)

  const prompt = useCallback(
    (request: PromptRequest) =>
      new Promise<string | null>((resolve) => setPending({ ...request, resolve })),
    [],
  )

  const dialog = pending ? (
    <Dialog request={pending} onDone={() => setPending(null)} />
  ) : null

  return { prompt, dialog }
}
