/**
 * 把任意 reject 值转成可读文案。
 *
 * Tauri 的 invoke 失败时抛出的是序列化后的普通对象（`{ code, message }`），
 * 不是 Error 实例，直接 `String(err)` 只会得到 `[object Object]`。
 */
/**
 * 取出后端 `GitError` 的 code，拿不到就是 null。
 *
 * 用来区分「这次失败要不要给用户一个升级动作」——比如删 worktree 撞上
 * `worktree_dirty` 时，界面要把按钮换成「仍要删除」，而不是单纯报错。
 * 靠匹配 message 文案做这件事太脆，改一个字就失效。
 */
export function toErrorCode(err: unknown): string | null {
  if (err && typeof err === 'object') {
    const e = err as { code?: unknown }
    if (typeof e.code === 'string') return e.code
  }
  return null
}

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
