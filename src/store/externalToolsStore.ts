import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useCallback, useEffect, useRef, useState } from 'react'
import { toErrorMessage } from '@/engine/errors'
import type { DiffTarget, ExternalToolsConfig } from '@/engine/externalTools'
import { samePath } from '@/engine/path'

/** 外部工具退出事件 `external-tool-exited` 的内容，与 Rust 侧 `ExternalToolExited` 一一对应。 */
export interface ExternalToolExited {
  worktree: string
  file: string
  toolLabel: string
  /** 被信号杀掉时没有退出码。 */
  code: number | null
  /** 等待进程本身失败时的原因。 */
  error: string | null
}

/**
 * 读取外部 diff / merge 工具配置。
 *
 * 不监听配置文件，改为窗口重新获得焦点时重读：用户点「编辑外部工具配置…」去别的编辑器里改完、
 * 切回来，菜单就是新的。只是读一个小 JSON，比文件监听便宜，也不和 `repo-changed` 抢重入。
 * 配置坏了时 `config` 为 null，菜单据此只留「编辑外部工具配置…」，
 * 报错信息（后端已带上配置文件路径）放在 `error` 里。
 */
export function useExternalTools() {
  const [config, setConfig] = useState<ExternalToolsConfig | null>(null)
  const [error, setError] = useState<string | null>(null)

  const reload = useCallback(async () => {
    try {
      setConfig(await invoke<ExternalToolsConfig>('read_external_tools'))
      setError(null)
    } catch (err) {
      setConfig(null)
      setError(toErrorMessage(err))
    }
  }, [])

  useEffect(() => {
    void reload()
    const onFocus = () => { void reload() }
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [reload])

  const openConfig = useCallback(async () => {
    await invoke('open_external_tools_config')
  }, [])

  return { config, error, reload, openConfig }
}

/** 用外部工具对比一个文件。立即返回；工具退出后后端发 `external-tool-exited`。 */
export function openExternalDiff(path: string, toolId: string, target: DiffTarget): Promise<void> {
  return invoke('open_external_diff', { path, toolId, target })
}

/**
 * 订阅外部工具退出事件，只认 `path` 这个仓库的（多开标签时别的仓库不受影响）。
 *
 * `onExit` 放进 ref：调用方每次渲染都传新闭包，直接放进依赖会让监听反复注销重注册，
 * 中间那一瞬间到的事件就丢了。
 */
export function useExternalToolExit(path: string | null, onExit: (payload: ExternalToolExited) => void) {
  const handler = useRef(onExit)
  handler.current = onExit

  useEffect(() => {
    if (!path) return
    let unlisten: (() => void) | undefined
    let disposed = false
    void listen<ExternalToolExited>('external-tool-exited', (event) => {
      if (samePath(event.payload.worktree, path)) handler.current(event.payload)
    }).then((fn) => {
      if (disposed) fn()
      else unlisten = fn
    })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [path])
}
