/**
 * 归一化路径用于比较。
 *
 * git CLI 输出正斜杠，而 Tauri dialog 与硬编码常量给的是反斜杠，
 * 裸字符串比较永远不相等。统一为正斜杠、去掉尾部斜杠，
 * 并忽略大小写（本应用面向 Windows）。
 */
export function normalizePath(path: string): string {
  return path.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase()
}

/** 两个路径是否指向同一位置（仅字符串归一化，不解析符号链接）。 */
export function samePath(a: string, b: string): boolean {
  return normalizePath(a) === normalizePath(b)
}

/**
 * 取路径最后一段作为显示名。
 *
 * 和 `normalizePath` 不同，这里**不能**转小写：这是给人看的名字，
 * 仓库目录名的大小写要原样保留。
 */
export function basename(path: string): string {
  const parts = path.replace(/\\/g, '/').replace(/\/+$/, '').split('/')
  return parts[parts.length - 1] || path
}
