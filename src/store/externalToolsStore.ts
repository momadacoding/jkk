import { invoke } from '@tauri-apps/api/core'
import { useCallback, useEffect, useState } from 'react'
import { toErrorMessage } from '@/engine/errors'
import type { ExternalToolsConfig } from '@/engine/externalTools'

/**
 * 读取外部 diff / merge 工具配置。
 *
 * 和自定义命令一样不监听配置文件：用户改完重新加载即可。
 * 配置坏了时 `config` 为 null，菜单据此不给工具项，只留「编辑外部工具配置…」，
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
  }, [reload])

  const openConfig = useCallback(async () => {
    await invoke('open_external_tools_config')
  }, [])

  return { config, error, reload, openConfig }
}
