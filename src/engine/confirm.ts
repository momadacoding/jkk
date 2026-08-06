import { confirm } from '@tauri-apps/plugin-dialog'

/**
 * 弹一个确认框，返回用户是否点了确定。
 *
 * 必须用插件导出的 `confirm` 并 await：Tauri 的 dialog 插件把全局 `window.confirm`
 * 换成了 async 函数，`!window.confirm(...)` 拿到的是 Promise——恒为真值，守卫会
 * 静默失效。这个坑踩过一次（强推确认框从来没拦住过任何东西），所以收在这里，
 * 不要在各处重新写一遍。
 *
 * 弹框本身失败（权限没开、插件没注册）时一律当作「没确认」。用到它的都是不可逆
 * 操作，宁可什么都不做，也不能在没问过的情况下动手。
 */
export async function askConfirm(message: string, title = 'Confirm'): Promise<boolean> {
  try {
    return await confirm(message, { title, kind: 'warning' })
  } catch (err) {
    console.error('确认框弹不出来，已按「取消」处理：', err)
    return false
  }
}
