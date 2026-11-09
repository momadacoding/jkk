import type { DiffLine } from './types'

/** 左右分栏的一行：左边是旧版，右边是新版；某一侧没有对应行时为 null（画成空白占位）。 */
export interface SplitRow {
  left: DiffLine | null
  right: DiffLine | null
}

/**
 * 一个 hunk 的行排成左右两栏。
 *
 * 上下文行两边各放一份；一段连续的删除和新增按顺序一一对上（第 1 条删除对第 1 条新增……），
 * 多出来的那一侧另一边留空。与 Fork / GitHub 的分栏一致。
 */
export function toSplitRows(lines: readonly DiffLine[]): SplitRow[] {
  const rows: SplitRow[] = []
  let dels: DiffLine[] = []
  let adds: DiffLine[] = []
  const flush = () => {
    const n = Math.max(dels.length, adds.length)
    for (let i = 0; i < n; i++) rows.push({ left: dels[i] ?? null, right: adds[i] ?? null })
    dels = []
    adds = []
  }
  for (const line of lines) {
    if (line.origin === '-') dels.push(line)
    else if (line.origin === '+') adds.push(line)
    else {
      flush()
      rows.push({ left: line, right: line })
    }
  }
  flush()
  return rows
}

/** 行内容的一段：`mark` 非空时这段是不可见字符，`mark` 是要画出来的符号。 */
export interface ContentSegment {
  text: string
  mark: 'space' | 'tab' | 'cr' | null
}

/**
 * 「显示不可见字符」：把空格、制表符、行尾 `\r` 切成单独的段，组件给它们画上 `·` `→` `␍`。
 * 关着时只去掉行尾的 `\r`——`white-space: pre` 下它画不出东西，留着只会让复制出去的文本多一个字符。
 */
export function contentSegments(content: string, showInvisibles: boolean): ContentSegment[] {
  const hasCr = content.endsWith('\r')
  const body = hasCr ? content.slice(0, -1) : content
  if (!showInvisibles) return [{ text: body, mark: null }]
  const out: ContentSegment[] = []
  let plain = ''
  for (const ch of body) {
    if (ch === ' ' || ch === '\t') {
      if (plain) out.push({ text: plain, mark: null })
      plain = ''
      out.push({ text: ch, mark: ch === ' ' ? 'space' : 'tab' })
    } else {
      plain += ch
    }
  }
  if (plain) out.push({ text: plain, mark: null })
  if (hasCr) out.push({ text: '', mark: 'cr' })
  return out
}
