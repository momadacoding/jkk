import type { FileChange, StatusData } from './types'

/**
 * `read_status_paths` 的结果：Stage / Unstage 之后只重读刚操作的那几个路径（2026-11-05 排查报告 6.13 方案 A）。
 *
 * - `staged`：整份 Staged，与全量读的相同（只比 HEAD 和 index，不碰工作区）
 * - `unstaged`：只有点名的那几个路径的项，按 git 的顺序
 * - `ignoreCase`：index 是否不区分大小写，决定列表的排序
 */
export interface StatusPatch {
  staged: FileChange[]
  unstaged: FileChange[]
  ignoreCase: boolean
}

/**
 * 与 libgit2 列表相同的路径顺序：按完整路径逐字节比，`/` 也只是一个字节；
 * 不区分大小写时只折叠 ASCII 字母（`git__strcasecmp`）。
 *
 * 按码位比而不按 UTF-16 码元比：码位顺序就是 UTF-8 的字节序，码元比在 BMP 以外的字符上会排反。
 */
export function gitPathCompare(a: string, b: string, ignoreCase: boolean): number {
  const fold = (c: number) => (ignoreCase && c >= 0x41 && c <= 0x5a ? c + 0x20 : c)
  // 逐码位走两个字符串，不分配数组：合并一次要比上万次，跑在界面线程上
  let i = 0
  let j = 0
  while (i < a.length && j < b.length) {
    const x = a.codePointAt(i)!
    const y = b.codePointAt(j)!
    if (x !== y) {
      const fx = fold(x)
      const fy = fold(y)
      if (fx !== fy) return fx - fy
    }
    i += x > 0xffff ? 2 : 1
    j += y > 0xffff ? 2 : 1
  }
  return (a.length - i) - (b.length - j)
}

const sameChange = (x: FileChange, y: FileChange) =>
  x.path === y.path && x.status === y.status && x.additions === y.additions && x.deletions === y.deletions

/**
 * 把局部重读的结果合进上一次的状态。
 *
 * 为什么只换点名的那几项就够：Stage / Unstage 只改 index 里这几个路径，工作区不动，别的路径的
 * Unstaged（index 对比工作区）两边都没变。例外是 git 的连带改动（`git add t/x` 会把 index 里的
 * 文件 `t` 删掉，`status_paths_contract.rs` 实测），它一定会让没点名的路径在 Staged 里也变：
 * 发现这种情况就交回 `null`，调用方退回全量重读。
 *
 * 新项按 git 的顺序插回去，不追加到末尾：全量重读时列表就是这个顺序，不然下一次刷新列表会跳。
 *
 * @param touched 刚 Stage / Unstage 的路径
 * @returns 合并后的状态；有连带改动时为 `null`
 */
export function applyStatusPatch(prev: StatusData, touched: string[], patch: StatusPatch): StatusData | null {
  const named = new Set(touched)
  // 后端只交回点名路径的项；万一带出了别的（libgit2 的字面量 pathspec 仍按目录前缀匹配），
  // 合并会让它和列表里原有的那一项各留一份
  if (patch.unstaged.some((f) => !named.has(f.path))) return null
  const before = prev.staged.filter((f) => !named.has(f.path))
  const after = patch.staged.filter((f) => !named.has(f.path))
  if (before.length !== after.length || before.some((f, i) => !sameChange(f, after[i]))) return null

  // 两份都按 git 的顺序，归并一次就排好了
  const rest = prev.unstaged.filter((f) => !named.has(f.path))
  const fresh = patch.unstaged
  const unstaged: FileChange[] = []
  let i = 0
  let j = 0
  while (i < rest.length || j < fresh.length) {
    if (j >= fresh.length || (i < rest.length && gitPathCompare(rest[i].path, fresh[j].path, patch.ignoreCase) <= 0)) {
      unstaged.push(rest[i++])
    } else {
      unstaged.push(fresh[j++])
    }
  }
  return { staged: patch.staged, unstaged }
}
