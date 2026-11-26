import type { CommitNode, RefPointer } from './types'

/**
 * 提交图的搜索：在**已加载**的提交上匹配，命中的行高亮、跳转，不过滤掉行。
 *
 * 不过滤是有意的：命中之外的行一拿掉，父子关系就断了，图上的线没法画对。
 * 只搜已加载的提交：git 侧最多 5000 条、且只有标题（`graph.rs` 的 `summary()`），
 * 正文和更早的历史搜不到。
 */

/** hash 至少这么多位才参与匹配：再短的话 `add`、`a` 这种输入会命中一堆 hash。 */
const MIN_HASH_PREFIX = 4

/** 一条提交可被搜到的内容，按图上的行序与 `commits` 一一对应。 */
export interface SearchEntry {
  /**
   * 标题、作者、ref 名，全部小写，用换行隔开。
   *
   * 词是按空白拆出来的、不含换行，所以不会跨字段拼出命中（标题结尾 + 作者开头）。
   */
  text: string
  /** 小写的完整 oid：自己的，以及 SVN 节点收着的已交付本地提交的。只按前缀匹配。 */
  hashes: string[]
  /** SVN 节点的 revision；git 提交为 null。 */
  svnRev: number | null
}

/** 按空白拆词并转小写。空输入得到空数组，表示「没在搜」，不是「全部命中」。 */
export function parseSearch(raw: string): string[] {
  return raw.toLowerCase().split(/\s+/).filter(Boolean)
}

/** 给图上每一行建一次搜索内容；提交和 refs 不变时复用，按键时只做匹配。 */
export function buildSearchIndex(commits: CommitNode[], refs: RefPointer[]): SearchEntry[] {
  const refNames = new Map<string, string[]>()
  for (const r of refs) {
    const list = refNames.get(r.hash)
    if (list) list.push(r.name)
    else refNames.set(r.hash, [r.name])
  }
  return commits.map((c) => {
    const isSvn = c.vcs === 'svn'
    // 已交付的本地提交不单独画在图上，搜到它们就是搜到收着它们的这个 svn 节点
    const folded = c.deliveredFrom ?? []
    const parts = [c.message, c.author, ...(refNames.get(c.id) ?? [])]
    for (const f of folded) parts.push(f.message, f.author)
    return {
      text: parts.join('\n').toLowerCase(),
      // svn 节点的 id 是 `svn:1234`，不是 hash
      hashes: [...(isSvn ? [] : [c.id]), ...folded.map((f) => f.id)].map((h) => h.toLowerCase()),
      svnRev: isSvn && c.svnRev !== undefined ? c.svnRev : null,
    }
  })
}

function termMatches(entry: SearchEntry, term: string): boolean {
  if (entry.text.includes(term)) return true
  if (term.length >= MIN_HASH_PREFIX && entry.hashes.some((h) => h.startsWith(term))) return true
  if (entry.svnRev !== null) {
    const rev = String(entry.svnRev)
    if (term === rev || term === `r${rev}`) return true
  }
  return false
}

/** 每个词都命中同一条提交才算（AND）。返回命中行在 `index` 里的下标，按行序。 */
export function findMatches(index: SearchEntry[], terms: string[]): number[] {
  if (terms.length === 0) return []
  const out: number[] = []
  index.forEach((entry, i) => {
    if (terms.every((t) => termMatches(entry, t))) out.push(i)
  })
  return out
}

/**
 * 从 `current` 行出发找下一个（`dir = 1`，往下、更早）或上一个（`dir = -1`）命中，走到头绕回。
 *
 * `inclusive`：当前行本身就是命中时留在原地。输入时用它：多打一个字、当前那条仍然命中的话，
 * 选中不该往下跳。`current = -1` 表示没有选中。
 */
export function stepMatch(matches: number[], current: number, dir: 1 | -1, inclusive: boolean): number | null {
  if (matches.length === 0) return null
  if (current < 0) return dir === 1 ? matches[0] : matches[matches.length - 1]
  if (dir === 1) {
    return matches.find((m) => (inclusive ? m >= current : m > current)) ?? matches[0]
  }
  for (let i = matches.length - 1; i >= 0; i--) {
    if (inclusive ? matches[i] <= current : matches[i] < current) return matches[i]
  }
  return matches[matches.length - 1]
}

/** 当前行是第几个命中（从 1 起），给「2 / 17」这样的计数用；不是命中时为 0。 */
export function matchPosition(matches: number[], current: number): number {
  return matches.indexOf(current) + 1
}

/** 只取判断快捷键所需的字段，不依赖 DOM。 */
export interface SearchKeyEvent {
  key: string
  /** 物理键位（`KeyF`），不随键盘布局变。只在 key 不是拉丁字母时才看它。 */
  code?: string
  ctrlKey: boolean
  metaKey: boolean
  shiftKey: boolean
  altKey: boolean
  /** 按住不放产生的自动重复。 */
  repeat?: boolean
  /** 输入法正在组词。 */
  isComposing?: boolean
}

/**
 * 窗口级的搜索快捷键：Ctrl+F（macOS 上 Cmd+F）聚焦搜索框，F3 / Shift+F3 在命中间跳。
 *
 * - 输入法组词时一律不管：这时的按键属于输入法。
 * - 按住 F3 的自动重复不算：每跳一下都会读那条提交的 diff，按住两秒就是几十次读取。
 * - Ctrl+Shift+F、Ctrl+Alt+F 留着，以后可能另有含义。
 * - 俄文、希腊文等布局下 Ctrl+F 拿到的 key 是本地字母，这时才退回看物理键位；不能一律看 code，
 *   AZERTY 之类的布局上 KeyF 那个键可能是别的拉丁字母（同 `isSelectAllShortcut`）。
 */
export function windowSearchKey(e: SearchKeyEvent, searching: boolean): 'focus' | 'next' | 'prev' | null {
  if (e.isComposing) return null
  if (e.ctrlKey || e.metaKey) {
    if (e.shiftKey || e.altKey) return null
    if (e.key.toLowerCase() === 'f') return 'focus'
    return e.code === 'KeyF' && !/^[\x20-\x7e]$/.test(e.key) ? 'focus' : null
  }
  if (e.key === 'F3' && searching && !e.altKey && !e.repeat) return e.shiftKey ? 'prev' : 'next'
  return null
}

/**
 * 搜索框里的按键：Enter 下一个、Shift+Enter 上一个、Esc。
 *
 * 输入法组词时 Enter 是把拼音原样上屏、Esc 是取消候选，都不归搜索管——不排除的话，
 * 打中文的人每上屏一次就跳一个命中，取消候选会把整个搜索词清掉。按住 Enter 的自动重复同 F3。
 */
export function inputSearchKey(e: SearchKeyEvent): 'next' | 'prev' | 'escape' | null {
  if (e.isComposing || e.repeat) return null
  if (e.key === 'Enter' && !e.ctrlKey && !e.metaKey && !e.altKey) return e.shiftKey ? 'prev' : 'next'
  if (e.key === 'Escape') return 'escape'
  return null
}
