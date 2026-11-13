import { execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { describe, expect, it } from 'vitest'
import { AROUND_AFTER_MS, AROUND_BEFORE_MS, around, parseLines, percentile, summarize } from '../../scripts/perf-summary.mjs'

const T0 = 1_800_000_000_000

/** 一次提交前后的一小段记录，形状与真实文件相同。 */
const lines = [
  { ts: T0 - 10_000, dur: 0, src: 'rust', kind: 'meta', name: 'start', ok: true, meta: { version: '0.8.0' } },
  // 窗口外（开始前 0.5 s 之前）
  { ts: T0 - AROUND_BEFORE_MS - 1, dur: 30, src: 'rust', kind: 'proc', name: 'svn status', ok: true },
  // 窗口内、span 开始之前
  { ts: T0 - 100, dur: 0, src: 'web', kind: 'mark', name: 'focus', ok: true },
  { ts: T0, dur: 900, src: 'web', kind: 'span', name: 'commit', ok: true, meta: { staged: 3, amend: false } },
  { ts: T0 + 1, dur: 300, src: 'web', kind: 'span', name: 'commit.invoke', ok: true },
  { ts: T0 + 2, dur: 290, src: 'web', kind: 'invoke', name: 'commit', ok: true },
  { ts: T0 + 5, dur: 255, src: 'rust', kind: 'proc', name: 'git commit', ok: true, meta: { flags: ['-F', '--'] } },
  { ts: T0 + 310, dur: 500, src: 'web', kind: 'span', name: 'commit.refreshChanges', ok: true },
  { ts: T0 + 311, dur: 480, src: 'web', kind: 'invoke', name: 'read_status', ok: true, meta: { 'out.staged': 0 } },
  // 结束后、窗口内
  { ts: T0 + 900 + AROUND_AFTER_MS, dur: 10, src: 'rust', kind: 'proc', name: 'svn info', ok: false },
  // 窗口外
  { ts: T0 + 900 + AROUND_AFTER_MS + 1, dur: 10, src: 'rust', kind: 'proc', name: 'svn info', ok: true },
]
const text = `${lines.map((l) => JSON.stringify(l)).join('\n')}\n{"ts":1,"dur":`

describe('parseLines', () => {
  it('跳过空行和写了一半的最后一行', () => {
    const { records, bad } = parseLines(`${text}\n\n  \nnot json\n{"x":1}\n{"ts":1,"dur":2,"name":"no-kind"}`)
    expect(records).toHaveLength(lines.length)
    expect(bad).toBe(4)
  })
})

describe('percentile', () => {
  it('最近秩', () => {
    const s = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    expect(percentile(s, 0.5)).toBe(5)
    expect(percentile(s, 0.95)).toBe(10)
    expect(percentile([7], 0.95)).toBe(7)
    expect(percentile([], 0.5)).toBe(0)
  })
})

describe('summarize', () => {
  it('按 kind + name 分组、按 total 降序，meta 头不进表，失败单独计数', () => {
    const rows = summarize(parseLines(text).records)
    expect(rows.map((r: { kind: string; name: string }) => `${r.kind} ${r.name}`)).toEqual([
      'span commit',
      'span commit.refreshChanges',
      'invoke read_status',
      'span commit.invoke',
      'invoke commit',
      'proc git commit',
      'proc svn status',
      'proc svn info',
      'mark focus',
    ])
    const svnInfo = rows.find((r: { name: string }) => r.name === 'svn info')
    expect(svnInfo).toMatchObject({ count: 2, failed: 1, max: 10, total: 20 })
  })
})

describe('around', () => {
  it('取 span 开始前 0.5 s 到结束后 3 s，按开始时刻排，深度是包住它的 span 数', () => {
    const [w] = around(parseLines(text).records, 'commit')
    expect(w.ts).toBe(T0)
    expect(w.meta).toEqual({ staged: 3, amend: false })
    const view = w.rows.map((r: { rel: number; depth: number; kind: string; name: string }) => [r.rel, r.depth, r.kind, r.name])
    expect(view).toEqual([
      [-100, 0, 'mark', 'focus'],
      [0, 0, 'span', 'commit'],
      [1, 1, 'span', 'commit.invoke'],
      [2, 2, 'invoke', 'commit'],
      [5, 2, 'proc', 'git commit'],
      [310, 1, 'span', 'commit.refreshChanges'],
      [311, 2, 'invoke', 'read_status'],
      [900 + AROUND_AFTER_MS, 0, 'proc', 'svn info'],
    ])
    expect(w.rows.at(-1).ok).toBe(false)
  })

  it('开始时刻、耗时相同时 span 在外层；子段结束晚出不到 1 ms 仍算在里面（2026-11-12 手测的真实形状）', () => {
    const recs = [
      { ts: T0, dur: 131.4, src: 'web', kind: 'span', name: 'commit', ok: true },
      // 先写入的是 invoke（它先结束），同一毫秒开始、同样耗时
      { ts: T0, dur: 120.8, src: 'web', kind: 'invoke', name: 'commit', ok: true },
      { ts: T0, dur: 120.8, src: 'web', kind: 'span', name: 'commit.invoke', ok: true },
      { ts: T0 + 127, dur: 4.7, src: 'web', kind: 'invoke', name: 'read_status', ok: true },
      { ts: T0 + 127, dur: 4.7, src: 'web', kind: 'span', name: 'commit.refreshChanges', ok: true },
    ]
    const [w] = around(recs, 'commit')
    expect(w.rows.map((r: { depth: number; kind: string; name: string }) => [r.depth, r.kind, r.name])).toEqual([
      [0, 'span', 'commit'],
      [1, 'span', 'commit.invoke'],
      [2, 'invoke', 'commit'],
      [1, 'span', 'commit.refreshChanges'],
      [2, 'invoke', 'read_status'],
    ])
  })

  it('没有这个 span 时为空', () => {
    expect(around(parseLines(text).records, 'nope')).toEqual([])
  })
})

describe('脚本文件', () => {
  it('开头没有 shebang：本地 LF 检出时 Vite 能吃下，CI 的 CRLF 检出会在导入时报 SyntaxError', () => {
    const source = readFileSync(path.resolve(__dirname, '../../scripts/perf-summary.mjs'), 'utf8')
    expect(source.startsWith('#!')).toBe(false)
  })
})

describe('命令行', () => {
  it('默认出表，--around 出时间线，--json 出 JSON，参数不对退出 2', () => {
    const dir = mkdtempSync(path.join(tmpdir(), 'perf-summary-'))
    try {
      const file = path.join(dir, 'p.jsonl')
      writeFileSync(file, text)
      const script = path.resolve(__dirname, '../../scripts/perf-summary.mjs')
      const run = (...args: string[]) => execFileSync(process.execPath, [script, ...args], { encoding: 'utf8' })

      const table = run(file)
      expect(table).toContain('跳过 1 行坏行')
      expect(table).toMatch(/span commit\s+1\s+0\s+900\.0/)

      const timeline = run(file, '--around', 'commit')
      expect(timeline).toContain('== commit #1')
      expect(timeline).toMatch(/5\.0\s+255\.0\s+ {4}proc\s+git commit/)

      const json = JSON.parse(run(file, '--around', 'commit', '--json'))
      expect(json.records).toBe(lines.length)
      expect(json.around[0].rows).toHaveLength(8)

      expect(() => run()).toThrow()
      expect(() => run(file, '--around')).toThrow()
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
