/**
 * 性能记录模式（`--perf`）的纯逻辑：记录对象、数量摘要、批量缓冲。
 *
 * 设计见 `docs/superpowers/specs/2026-11-12-performance-monitor-design.md` §4.2 / §4.4。
 * 这里不碰 Tauri、不碰 DOM，副作用（时钟、发送）都由调用方注入。
 */

export type PerfKind = 'invoke' | 'span' | 'mark'

/** meta 里只放数字、布尔和短的固定字符串（设计 §4.2）。 */
export type PerfMeta = Record<string, number | boolean | string>

/** 一条前端记录，字段与 Rust 侧 `engine::perf::Record` 相同。 */
export interface PerfRecord {
  ts: number
  dur: number
  src: 'web'
  kind: PerfKind
  name: string
  ok: boolean
  meta?: PerfMeta
}

/**
 * 一个值里每个**顶层**数组字段的长度，键加上 `prefix`。值本身是数组时记成 `<prefix>length`。
 *
 * 只取 `.length`，不看元素、不序列化：参数和返回值里有路径、提交说明，不能进记录（设计 §4.5）。
 */
export function arrayCounts(value: unknown, prefix: string): PerfMeta {
  const out: PerfMeta = {}
  if (Array.isArray(value)) {
    out[`${prefix}length`] = value.length
    return out
  }
  if (value === null || typeof value !== 'object') return out
  for (const [key, field] of Object.entries(value)) {
    if (Array.isArray(field)) out[`${prefix}${key}`] = field.length
  }
  return out
}

/** 一条 invoke 记录：参数里的数组长度记成 `in.<键>`，返回值里的记成 `out.<键>`。 */
export function invokeRecord(
  cmd: string,
  ts: number,
  dur: number,
  ok: boolean,
  args: unknown,
  result: unknown,
): PerfRecord {
  const meta = { ...arrayCounts(args, 'in.'), ...(ok ? arrayCounts(result, 'out.') : {}) }
  return withMeta({ ts, dur, src: 'web', kind: 'invoke', name: cmd, ok }, meta)
}

export function spanRecord(name: string, ts: number, dur: number, ok: boolean, meta?: PerfMeta): PerfRecord {
  return withMeta({ ts, dur, src: 'web', kind: 'span', name, ok }, meta)
}

export function markRecord(name: string, ts: number, meta?: PerfMeta): PerfRecord {
  return withMeta({ ts, dur: 0, src: 'web', kind: 'mark', name, ok: true }, meta)
}

function withMeta(rec: PerfRecord, meta: PerfMeta | undefined): PerfRecord {
  return meta && Object.keys(meta).length > 0 ? { ...rec, meta } : rec
}

/** 攒满这么多条就立刻发一批，不等定时器（设计 §4.4）。 */
export const PERF_BATCH_MAX = 100

/**
 * 待发的记录。满 [`PERF_BATCH_MAX`] 条立刻交给 `send`，其余由调用方定时 `flush`。
 *
 * 交出去之前先从缓冲里拿掉：发送失败就丢了，不重试——记录模式出问题不能反过来拖住业务。
 */
export class PerfBuffer {
  private lines: string[] = []

  constructor(private readonly send: (lines: string[]) => void) {}

  get size(): number {
    return this.lines.length
  }

  push(rec: PerfRecord): void {
    this.lines.push(JSON.stringify(rec))
    if (this.lines.length >= PERF_BATCH_MAX) this.flush()
  }

  flush(): void {
    if (this.lines.length === 0) return
    const batch = this.lines
    this.lines = []
    this.send(batch)
  }
}
