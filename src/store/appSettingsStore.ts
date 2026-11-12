import { invoke } from '@/store/ipc'
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

/**
 * 告诉后端设置框是否正在录快捷键。录制期间按下全局快捷键只呼出、不隐藏：
 * 重录同一个组合键时系统会先把这次按键当成全局快捷键派过来，切换的话窗口当场消失。
 *
 * 失败只记日志：最坏情况是录制时窗口被藏一下，再按一次就回来了，不值得打断用户。
 *
 * 必须串行发送：后端是 async 命令，各自落在线程池上，连着发出的 true / false 不保证按序执行。
 * 要是 false 先到、true 后到，后端会一直以为在录制，快捷键从此只呼出、再也不隐藏。
 */
let recordingQueue: Promise<unknown> = Promise.resolve()

export function setShortcutRecording(recording: boolean): Promise<void> {
  const next = recordingQueue.then(() => invoke('set_shortcut_recording', { recording })).then(
    () => undefined,
    (err: unknown) => {
      console.warn('set_shortcut_recording 失败', err)
    },
  )
  recordingQueue = next
  return next
}
