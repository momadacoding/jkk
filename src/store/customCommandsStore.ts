import { invoke } from '@tauri-apps/api/core'
import { useCallback, useEffect, useState } from 'react'
import type { CustomCommand, CustomCommandPlan } from '../engine/customCommands'
import { toErrorMessage } from '../engine/errors'

/**
 * 读取并执行用户自定义命令。
 *
 * 配置改动不做文件监听：多开标签时每个实例都监听同一个文件、
 * 又要和既有的 `repo-changed` 监听抢重入，不值得。用户改完点 Reload 即可。
 */
export function useCustomCommands() {
  const [commands, setCommands] = useState<CustomCommand[]>([])
  const [error, setError] = useState<string | null>(null)

  const reload = useCallback(async () => {
    try {
      const list = await invoke<CustomCommand[]>('read_custom_commands')
      setCommands(list)
      setError(null)
    } catch (err) {
      // 配置坏了就退化成没有命令，但把原因留在界面上
      setCommands([])
      setError(toErrorMessage(err))
    }
  }, [])

  useEffect(() => {
    void reload()
  }, [reload])

  const run = useCallback(async (plan: CustomCommandPlan) => {
    await invoke('run_custom_command', {
      program: plan.program,
      args: plan.args,
      cwd: plan.cwd,
    })
  }, [])

  const openConfig = useCallback(async () => {
    await invoke('open_custom_commands_config')
  }, [])

  return { commands, error, reload, run, openConfig }
}
