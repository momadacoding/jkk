import type { DiffHunk, DiffLine } from './types'

/**
 * 「忽略换行符」：把只差行尾 `\r` 的一删一增配成上下文行，再按上下文行数重新切 hunk。
 *
 * 为什么在前端做：libgit2 1.9 没有 `IGNORE_CR_AT_EOL`（xdiff 里有，但 libgit2 不映射），
 * `IGNORE_WHITESPACE_EOL` 又会连行尾空格的改动一起吞掉。libgit2 把行尾的 `\r` 留在行内容里
 * （`tests/diff_view_options.rs` 钉住了这一点），所以这里能分辨出来。
 *
 * 一整个文件从 LF 换成 CRLF 时，libgit2 给的是「全部删掉再全部加上」的一大块；
 * 块内按去掉 `\r` 之后的内容重新比一遍（Myers），配上的行就成了上下文。
 * 只有原样不同、去掉 `\r` 才相同的才藏：文件末尾补上换行（`\ No newline at end of file`）
 * 在行内容上两边一模一样，那不是 CRLF / LF 的差别，照样显示（独立 review 第 2 条）。
 */

/** 去掉行尾的一个 `\r`。 */
export function stripCr(content: string): string {
  return content.endsWith('\r') ? content.slice(0, -1) : content
}

/** 一块里改动太多时放弃重比、原样显示：Myers 的耗时和内存随改动数平方增长。 */
const MAX_EDITS = 500

type Op = { kind: '='; a: number; b: number } | { kind: '-'; a: number } | { kind: '+'; b: number }

/**
 * Myers 差分（带回溯）。两端相同的部分先剥掉，整文件换行符的情形基本在这一步就做完了。
 * 改动数超过 `maxEdits` 返回 null。
 */
export function diffSequences(a: readonly string[], b: readonly string[], maxEdits = MAX_EDITS): Op[] | null {
  let pre = 0
  while (pre < a.length && pre < b.length && a[pre] === b[pre]) pre++
  let suf = 0
  while (suf < a.length - pre && suf < b.length - pre && a[a.length - 1 - suf] === b[b.length - 1 - suf]) suf++
  const head: Op[] = Array.from({ length: pre }, (_, i) => ({ kind: '=', a: i, b: i }))
  const tail: Op[] = Array.from({ length: suf }, (_, i) => ({
    kind: '=', a: a.length - suf + i, b: b.length - suf + i,
  }))
  const n = a.length - pre - suf
  const m = b.length - pre - suf
  const mid = myers(n, m, (x, y) => a[pre + x] === b[pre + y], maxEdits)
  if (!mid) return null
  const shifted = mid.map((op): Op => {
    if (op.kind === '=') return { kind: '=', a: op.a + pre, b: op.b + pre }
    if (op.kind === '-') return { kind: '-', a: op.a + pre }
    return { kind: '+', b: op.b + pre }
  })
  return [...head, ...shifted, ...tail]
}

function myers(n: number, m: number, eq: (x: number, y: number) => boolean, maxEdits: number): Op[] | null {
  const limit = Math.min(n + m, maxEdits)
  const off = limit + 1
  const v = new Int32Array(2 * limit + 3)
  const trace: Int32Array[] = []
  for (let d = 0; d <= limit; d++) {
    trace.push(v.slice())
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1
      let y = x - k
      while (x < n && y < m && eq(x, y)) { x++; y++ }
      v[off + k] = x
      if (x >= n && y >= m) return backtrack(trace, n, m, off)
    }
  }
  return null
}

function backtrack(trace: Int32Array[], n: number, m: number, off: number): Op[] {
  const ops: Op[] = []
  let x = n
  let y = m
  for (let d = trace.length - 1; d >= 0; d--) {
    const v = trace[d]
    const k = x - y
    const prevK = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? k + 1 : k - 1
    const prevX = v[off + prevK]
    const prevY = prevX - prevK
    while (x > prevX && y > prevY) {
      ops.push({ kind: '=', a: x - 1, b: y - 1 })
      x--
      y--
    }
    if (d > 0) {
      if (x === prevX) ops.push({ kind: '+', b: y - 1 })
      else ops.push({ kind: '-', a: x - 1 })
    }
    x = prevX
    y = prevY
  }
  return ops.reverse()
}

/**
 * 一块连续的删除/新增，按去掉 `\r` 之后的内容重新配对。配上的成为上下文行
 * （内容取新版那一行，两个行号都有）；剩下的照 git 的习惯，每段先删后增。
 */
function realignBlock(dels: DiffLine[], adds: DiffLine[]): { lines: DiffLine[]; matched: boolean } {
  if (dels.length === 0 || adds.length === 0) return { lines: [...dels, ...adds], matched: false }
  const ops = diffSequences(dels.map((l) => stripCr(l.content)), adds.map((l) => stripCr(l.content)))
  if (!ops) return { lines: [...dels, ...adds], matched: false }
  const out: DiffLine[] = []
  let pendingDel: DiffLine[] = []
  let pendingAdd: DiffLine[] = []
  let matched = false
  const flush = () => {
    out.push(...pendingDel, ...pendingAdd)
    pendingDel = []
    pendingAdd = []
  }
  for (const op of ops) {
    if (op.kind === '-') pendingDel.push(dels[op.a])
    else if (op.kind === '+') pendingAdd.push(adds[op.b])
    // 原样就相同的一对不是换行符改动：多半是文件末尾补上或去掉了换行（行内容里看不出 `\n`），
    // 或者 xdiff 给的不是最小改动。git --ignore-cr-at-eol 照样显示它们，这里也照样显示
    else if (dels[op.a].content === adds[op.b].content) {
      pendingDel.push(dels[op.a])
      pendingAdd.push(adds[op.b])
    } else {
      flush()
      matched = true
      const add = adds[op.b]
      out.push({ origin: ' ', content: add.content, oldLineno: dels[op.a].oldLineno, newLineno: add.newLineno })
    }
  }
  flush()
  return { lines: out, matched }
}

function realignHunk(lines: DiffLine[]): { lines: DiffLine[]; matched: boolean } {
  const out: DiffLine[] = []
  let dels: DiffLine[] = []
  let adds: DiffLine[] = []
  let matched = false
  const flush = () => {
    const r = realignBlock(dels, adds)
    out.push(...r.lines)
    matched ||= r.matched
    dels = []
    adds = []
  }
  for (const line of lines) {
    if (line.origin === '-') dels.push(line)
    else if (line.origin === '+') adds.push(line)
    else {
      flush()
      out.push(line)
    }
  }
  flush()
  return { lines: out, matched }
}

/** `@@ -a,b +c,d @@ 函数名` 里第二个 `@@` 之后的那段（git 的函数上下文）。 */
function headerSuffix(header: string): string {
  const m = /^@@ [^@]* @@(.*)$/.exec(header)
  return m ? m[1] : ''
}

function range(start: number, count: number): string {
  return count === 1 ? `${start}` : `${start},${count}`
}

/** 从一段行里切出一个 hunk。只有切出来的就是原来那整个 hunk 时才保留原样的头。 */
function sliceHunk(src: DiffHunk, lines: DiffLine[], from: number, to: number, whole: boolean): DiffHunk {
  const part = lines.slice(from, to)
  if (whole && part.length === src.lines.length && part.every((l, i) => l === src.lines[i])) return src
  // 起始行号：取这一段里第一个有该侧行号的行；一个都没有（纯新增、上下文为 0）时按 git 的写法，
  // 用前一行的行号
  const startOf = (key: 'oldLineno' | 'newLineno', fallback: number) => {
    const first = part.find((l) => l[key] !== null)
    if (first) return first[key] as number
    for (let i = from - 1; i >= 0; i--) {
      const n = lines[i][key]
      if (n !== null) return n
    }
    return fallback === 0 ? 0 : fallback - 1
  }
  const oldCount = part.filter((l) => l.oldLineno !== null).length
  const newCount = part.filter((l) => l.newLineno !== null).length
  const oldStart = startOf('oldLineno', src.oldStart)
  const newStart = startOf('newLineno', src.newStart)
  const suffix = from === 0 ? headerSuffix(src.header) : ''
  return {
    oldStart,
    newStart,
    header: `@@ -${range(oldStart, oldCount)} +${range(newStart, newCount)} @@${suffix}`,
    lines: part,
  }
}

/** 按上下文行数重新切：改动前后各留 `context` 行，两段上下文挨上或重叠就并成一个 hunk（同 git）。 */
function rehunk(src: DiffHunk, lines: DiffLine[], context: number): DiffHunk[] {
  const changed: number[] = []
  lines.forEach((l, i) => { if (l.origin !== ' ') changed.push(i) })
  if (changed.length === 0) return []
  const spans: Array<[number, number]> = []
  for (const i of changed) {
    const from = Math.max(0, i - context)
    const to = Math.min(lines.length, i + 1 + context)
    const last = spans[spans.length - 1]
    if (last && from <= last[1]) last[1] = Math.max(last[1], to)
    else spans.push([from, to])
  }
  const whole = spans.length === 1 && spans[0][0] === 0 && spans[0][1] === lines.length
  return spans.map(([from, to]) => sliceHunk(src, lines, from, to, whole))
}

export interface EolResult {
  hunks: DiffHunk[]
  /** 有没有改动因为只差换行符而被藏起来。整个文件都只是换行符改动时，界面据此说明为什么是空的。 */
  hidden: boolean
}

/**
 * 把只差换行符的改动藏起来。`context` 是当前的上下文行数（整个文件时为 Infinity）。
 * 一个 hunk 里一处都没藏的话原样返回（同一个对象），不重切。
 */
export function ignoreEolChanges(hunks: readonly DiffHunk[], context: number): EolResult {
  const out: DiffHunk[] = []
  let hidden = false
  for (const hunk of hunks) {
    const r = realignHunk(hunk.lines)
    if (!r.matched) {
      out.push(hunk)
      continue
    }
    hidden = true
    out.push(...rehunk(hunk, r.lines, context))
  }
  return { hunks: out, hidden }
}
