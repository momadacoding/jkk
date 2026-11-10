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
 *
 * 连续的空格并成一段：每段是一个 `<span>`，8 个空格缩进的万行文件逐个切会多出几万个节点
 * （第二轮独立 review 第 6 条）。一段里每个空格一个点由 CSS 的重复背景画。制表符仍一个一段：
 * 它的宽度随所在列变，画不成等距的重复背景。
 */
export function contentSegments(content: string, showInvisibles: boolean): ContentSegment[] {
  const hasCr = content.endsWith('\r')
  const body = hasCr ? content.slice(0, -1) : content
  if (!showInvisibles) return [{ text: body, mark: null }]
  const out: ContentSegment[] = []
  let run = ''
  let runMark: ContentSegment['mark'] = null
  const flush = () => {
    if (run) out.push({ text: run, mark: runMark })
    run = ''
  }
  for (const ch of body) {
    const mark = ch === ' ' ? 'space' : ch === '\t' ? 'tab' : null
    if (mark !== runMark || mark === 'tab') {
      flush()
      runMark = mark
    }
    run += ch
  }
  flush()
  if (hasCr) out.push({ text: '', mark: 'cr' })
  return out
}
