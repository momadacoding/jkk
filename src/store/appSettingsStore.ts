import { invoke } from '@tauri-apps/api/core'
import type { AppSettingsView } from '@/engine/appSettings'

export function readAppSettings(): Promise<AppSettingsView> {
  return invoke<AppSettingsView>('read_app_settings')
}

/**
 * 保存应用设置。`showShortcut` 传 `null` 关掉快捷键。
 *
 * 新快捷键注册失败时后端会报错、保住旧的，磁盘上的设置不变。
 */
export function saveAppSettings(
  closeToTray: boolean,
  showShortcut: string | null,
): Promise<AppSettingsView> {
  return invoke<AppSettingsView>('save_app_settings', { closeToTray, showShortcut })
}
