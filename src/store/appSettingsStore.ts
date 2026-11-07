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

/** 打开 / 关掉 SVN 自动跟进（事故分析 6.12）。后端 `set_svn_auto_follow(enabled)`。 */
export function setSvnAutoFollow(enabled: boolean): Promise<AppSettingsView> {
  return invoke<AppSettingsView>('set_svn_auto_follow', { enabled })
}

/**
 * 设置框的「保存」：快捷键那一组走 `save_app_settings`，自动跟进单独一条命令，只在改了时发。
 *
 * 先存快捷键：它会失败（被别的程序占了），失败时自动跟进也不动，框里只有一条错误。
 */
export async function saveAllSettings(
  before: AppSettingsView,
  closeToTray: boolean,
  showShortcut: string | null,
  svnAutoFollow: boolean,
): Promise<AppSettingsView> {
  const view = await saveAppSettings(closeToTray, showShortcut)
  if (svnAutoFollow === before.svnAutoFollow) return view
  return setSvnAutoFollow(svnAutoFollow)
}
