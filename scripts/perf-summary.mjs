// 顶部不写 shebang：这个文件同时被 vitest 从 .ts 测试导入，CRLF 检出时 shebang 会让 Vite 解析报错（v0.8.2 发版预检就挂在这里）。
// 汇总 jkk 性能记录模式（`jkk.exe --perf`）写出的 JSONL。只用 Node 内置模块。
//
//   node scripts/perf-summary.mjs <file.jsonl>                 按 kind + name 出 count / p50 / p95 / max / total
//   node scripts/perf-summary.mjs <file.jsonl> --around commit 每次 commit 前 0.5 s 到结束后 3 s 的时间线
//   node scripts/perf-summary.mjs <file.jsonl> --json          把上面的结果按 JSON 输出
//
// 记录格式与用法见 README「排查：性能记录模式」与 docs/superpowers/specs/2026-11-12-performance-monitor-design.md。

import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

/** 时间线窗口：span 开始前多少毫秒、结束后多少毫秒（设计 §4.6）。 */
export const AROUND_BEFORE_MS = 500
export const AROUND_AFTER_MS = 3000

/**
 * 判断「包住」时结束时刻的余量。前端 `ts` 是 `Date.now()` 的整毫秒、`dur` 是 `performance.now()`
 * 的差值，子段的结束时刻可能比父段晚出不到 1 ms（2026-11-12 手测：`commit.refreshChanges` 被算到了
 * `commit` 外面）。
 */
export const END_SLACK_MS = 1

/** 开始时刻、耗时都相同时的次序：外层的 span 排前面，瞬时的 mark 排最后。 */
const KIND_ORDER = (r) => ({ span: 0, invoke: 1, proc: 2, mark: 3 })[r.kind] ?? 4

/** 逐行解析；坏行跳过并计数（进程被杀时最后一行可能只写了一半）。 */
export function parseLines(text) {
  const records = []
  let bad = 0
  for (const line of text.split(/\r?\n/)) {
    if (!line.trim()) continue
    try {
      const rec = JSON.parse(line)
      if (
        rec &&
        typeof rec.ts === 'number' &&
        typeof rec.dur === 'number' &&
        typeof rec.name === 'string' &&
        typeof rec.kind === 'string'
      ) {
        records.push(rec)
      } else {
        bad++
      }
    } catch {
      bad++
    }
  }
  return { records, bad }
}

/** 最近秩百分位：排好序的数组里第 ceil(p·n) 个。 */
export function percentile(sorted, p) {
  if (sorted.length === 0) return 0
  const rank = Math.max(1, Math.ceil(p * sorted.length))
  return sorted[Math.min(rank, sorted.length) - 1]
}

/** 按 `kind name` 分组，按 total 降序。`meta` 头不进表。 */
export function summarize(records) {
  const groups = new Map()
  for (const rec of records) {
    if (rec.kind === 'meta') continue
    const key = `${rec.kind} ${rec.name}`
    let g = groups.get(key)
    if (!g) {
      g = { kind: rec.kind, name: rec.name, durs: [], failed: 0 }
      groups.set(key, g)
    }
    g.durs.push(rec.dur)
    if (rec.ok === false) g.failed++
  }
  const rows = []
  for (const g of groups.values()) {
    const sorted = [...g.durs].sort((a, b) => a - b)
    rows.push({
      kind: g.kind,
      name: g.name,
      count: sorted.length,
      failed: g.failed,
      p50: percentile(sorted, 0.5),
      p95: percentile(sorted, 0.95),
      max: sorted[sorted.length - 1],
      total: sorted.reduce((a, b) => a + b, 0),
    })
  }
  return rows.sort((a, b) => b.total - a.total || a.name.localeCompare(b.name))
}

/**
 * 每个名为 `spanName` 的 span 一段时间线：它开始前 [`AROUND_BEFORE_MS`] 到结束后 [`AROUND_AFTER_MS`]
 * 内开始的所有记录，`rel` 是相对 span 开始的毫秒，`depth` 是包住它的 span 个数（用来缩进）。
 */
export function around(records, spanName) {
  const anchors = records.filter((r) => r.kind === 'span' && r.name === spanName)
  return anchors.map((anchor) => {
    const from = anchor.ts - AROUND_BEFORE_MS
    const to = anchor.ts + anchor.dur + AROUND_AFTER_MS
    const inWindow = records
      .filter((r) => r.kind !== 'meta' && r.ts >= from && r.ts <= to)
      .sort((a, b) => a.ts - b.ts || b.dur - a.dur || KIND_ORDER(a) - KIND_ORDER(b))
    const rows = inWindow.map((r, i) => ({
      rel: r.ts - anchor.ts,
      dur: r.dur,
      kind: r.kind,
      src: r.src,
      name: r.name,
      ok: r.ok !== false,
      meta: r.meta,
      // 排在它前面、把它包住的 span 个数。只看排在前面的，开始时刻和耗时都一样的两条
      // （`commit.invoke` 与它里面的 `invoke commit`）才不会互相包住
      depth: inWindow
        .slice(0, i)
        .filter((s) => s.kind === 'span' && s.ts <= r.ts && r.ts + r.dur <= s.ts + s.dur + END_SLACK_MS).length,
    }))
    return { ts: anchor.ts, dur: anchor.dur, meta: anchor.meta, rows }
  })
}

const ms = (v) => v.toFixed(1).padStart(8)

export function formatSummary(rows) {
  const nameWidth = Math.max(4, ...rows.map((r) => r.kind.length + 1 + r.name.length))
  const head = `${'name'.padEnd(nameWidth)}  ${'count'.padStart(6)} ${'fail'.padStart(5)} ${'p50'.padStart(8)} ${'p95'.padStart(8)} ${'max'.padStart(8)} ${'total'.padStart(10)}`
  const lines = [head, '-'.repeat(head.length)]
  for (const r of rows) {
    lines.push(
      `${`${r.kind} ${r.name}`.padEnd(nameWidth)}  ${String(r.count).padStart(6)} ${String(r.failed).padStart(5)} ${ms(r.p50)} ${ms(r.p95)} ${ms(r.max)} ${r.total.toFixed(1).padStart(10)}`,
    )
  }
  return lines.join('\n')
}

export function formatAround(windows, spanName) {
  if (windows.length === 0) return `没有名为 ${spanName} 的 span。`
  const out = []
  windows.forEach((w, i) => {
    const meta = w.meta ? ` ${JSON.stringify(w.meta)}` : ''
    out.push(`== ${spanName} #${i + 1}  ${new Date(w.ts).toISOString()}  ${w.dur.toFixed(1)} ms${meta}`)
    out.push(`${'+ms'.padStart(8)} ${'dur'.padStart(8)}  kind`)
    for (const r of w.rows) {
      const mark = r.ok ? ' ' : '✗'
      const meta = r.meta ? `  ${JSON.stringify(r.meta)}` : ''
      out.push(`${ms(r.rel)} ${ms(r.dur)} ${mark}${'  '.repeat(r.depth)}${r.kind.padEnd(7)}${r.name}${meta}`)
    }
    out.push('')
  })
  return out.join('\n')
}

function main(argv) {
  const usage = () => {
    console.error('用法：node scripts/perf-summary.mjs <file.jsonl> [--around <span 名>] [--json]')
    return 2
  }
  const args = argv.slice(2)
  let file
  let spanName
  let asJson = false
  for (let i = 0; i < args.length; i++) {
    const a = args[i]
    if (a === '--json') asJson = true
    else if (a === '--around') {
      spanName = args[++i]
      if (!spanName) return usage()
    } else if (!file && !a.startsWith('--')) file = a
    else return usage()
  }
  if (!file) return usage()
  const { records, bad } = parseLines(readFileSync(file, 'utf8'))
  const result = { file: path.basename(file), records: records.length, badLines: bad }
  if (spanName) result.around = around(records, spanName)
  else result.summary = summarize(records)

  if (asJson) {
    console.log(JSON.stringify(result, null, 2))
    return 0
  }
  console.log(`${result.file}: ${records.length} 条${bad ? `，跳过 ${bad} 行坏行` : ''}`)
  console.log(spanName ? formatAround(result.around, spanName) : formatSummary(result.summary))
  return 0
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
if (isMain) process.exitCode = main(process.argv)
