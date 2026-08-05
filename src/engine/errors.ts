/**
 * 把任意 reject 值转成可读文案。
 *
 * Tauri 的 invoke 失败时抛出的是序列化后的普通对象（`{ code, message }`），
 * 不是 Error 实例，直接 `String(err)` 只会得到 `[object Object]`。
 */
export function toErrorMessage(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  if (err && typeof err === 'object') {
    const e = err as { code?: unknown; message?: unknown }
    if (typeof e.message === 'string') {
      return typeof e.code === 'string' ? `[${e.code}] ${e.message}` : e.message
    }
    try {
      return JSON.stringify(err)
    } catch {
      return String(err)
    }
  }
  return String(err)
}
