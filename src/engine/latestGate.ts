/**
 * 「只认最后一次」的请求代次闸。
 *
 * 同一种刷新可能同时有好几个在飞：文件监听、外部工具退出、用户操作后各自发起一次。
 * 它们完成的先后不保证和发起的先后一致，晚到的旧结果会把新状态盖掉。
 * 发起时 `begin` 领一个号，写回前用 `isLatest` 核对，不是最新的就丢掉。
 *
 * key 不同的请求各自计数（status 和 diff 互不干扰）。
 */
export function createLatestGate(): {
  begin(key?: string): number
  isLatest(token: number, key?: string): boolean
} {
  const latest = new Map<string, number>()
  return {
    begin(key = '') {
      const next = (latest.get(key) ?? 0) + 1
      latest.set(key, next)
      return next
    },
    isLatest(token, key = '') {
      return latest.get(key) === token
    },
  }
}
