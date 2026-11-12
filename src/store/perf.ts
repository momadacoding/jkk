/**
 * 性能记录模式（`--perf`）的前端一侧：开关、打点、批量写回后端。
 *
 * 开关由后端在页面脚本执行前注入 `window.__JKK_PERF__`（决策日志 D1）。没有它（普通启动、
 * vitest）就是关闭：`perfSpan` 直接调原函数，`perfMark` 什么都不做，invoke 原样转发。
 * 后端记录文件没建成时，第一次 `perf_append` 回答 `false`，前端随即关掉自己（review 第 2 条）。
 */
import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import {
  PerfBuffer,
  invokeRecord,
  markRecord,
  spanRecord,
  type PerfMeta,
  type PerfRecord,
} from '@/engine/perfRecord'

/** 定时把缓冲发给后端的间隔（设计 §4.4）。 */
export const PERF_FLUSH_MS = 1000

export interface Perf {
  readonly enabled: boolean
  /**
   * 计一段异步过程。关闭时就是 `fn()`。抛错记 `ok: false`；`okOf` 给出时再按返回值判断
   * （`handleCommit` 失败时返回 `false` 而不抛）。
   */
  span<T>(name: string, fn: () => Promise<T>, meta?: PerfMeta, okOf?: (result: T) => boolean): Promise<T>
  /** 记一个瞬时事件。关闭时什么都不做。 */
  mark(name: string, meta?: PerfMeta): void
  /** 计一次 invoke 往返，结果和异常原样交回。只在打开时调用。 */
  timeInvoke<T>(cmd: string, args: unknown, run: () => Promise<T>): Promise<T>
  flush(): void
  /** 之后一律按关闭处理，丢掉没发的记录。 */
  disable(): void
}

export interface PerfDeps {
  enabled: boolean
  /** 记录的开始时刻：墙钟，与 Rust 侧 `SystemTime::now()` 同一基准。 */
  wall: () => number
  /** 量耗时用的单调时钟。 */
  mono: () => number
  send: (lines: string[]) => void
}

export function createPerf({ enabled: initial, wall, mono, send }: PerfDeps): Perf {
  let enabled = initial
  let buffer = new PerfBuffer(send)
  const push = (rec: PerfRecord) => {
    if (enabled) buffer.push(rec)
  }
  /** 开始时刻取墙钟、耗时取单调时钟：长时间运行、休眠唤醒、校时后两侧时间线仍对得上（review 第 1 条）。 */
  const start = () => ({ ts: wall(), t0: mono() })
  const timedSpan = async <T>(
    name: string,
    fn: () => Promise<T>,
    meta?: PerfMeta,
    okOf?: (result: T) => boolean,
  ): Promise<T> => {
    const { ts, t0 } = start()
    try {
      const result = await fn()
      push(spanRecord(name, ts, mono() - t0, okOf ? okOf(result) : true, meta))
      return result
    } catch (err) {
      push(spanRecord(name, ts, mono() - t0, false, meta))
      throw err
    }
  }
  return {
    get enabled() {
      return enabled
    },
    // 不是 async 函数：关闭时返回的就是 fn() 那个 Promise，不多包一层、不多等一拍
    span: <T>(name: string, fn: () => Promise<T>, meta?: PerfMeta, okOf?: (result: T) => boolean): Promise<T> =>
      enabled ? timedSpan(name, fn, meta, okOf) : fn(),
    mark(name: string, meta?: PerfMeta): void {
      if (enabled) push(markRecord(name, wall(), meta))
    },
    async timeInvoke<T>(cmd: string, args: unknown, run: () => Promise<T>): Promise<T> {
      const { ts, t0 } = start()
      try {
        const result = await run()
        push(invokeRecord(cmd, ts, mono() - t0, true, args, result))
        return result
      } catch (err) {
        push(invokeRecord(cmd, ts, mono() - t0, false, args, undefined))
        throw err
      }
    },
    flush: () => buffer.flush(),
    disable() {
      enabled = false
      buffer = new PerfBuffer(send)
    },
  }
}

const injected = (globalThis as { __JKK_PERF__?: unknown }).__JKK_PERF__ === true

/** 全局实例。写回后端走原始 invoke：这次调用本身不记录，也不 await（设计 §4.4）。 */
export const perf: Perf = createPerf({
  enabled: injected,
  wall: () => Date.now(),
  mono: () => performance.now(),
  send: (lines) => {
    void tauriInvoke<boolean>('perf_append', { lines })
      .then((written) => {
        if (written === false) perf.disable()
      })
      .catch(() => undefined)
  },
})

export const perfSpan = <T>(
  name: string,
  fn: () => Promise<T>,
  meta?: PerfMeta,
  okOf?: (result: T) => boolean,
): Promise<T> => perf.span(name, fn, meta, okOf)

export const perfMark = (name: string, meta?: PerfMeta): void => perf.mark(name, meta)

/**
 * 应用启动时调一次（`main.tsx`）：装定时 flush、窗口焦点打点、离开窗口时 flush。关闭时什么都不装。
 *
 * 失焦和隐藏时也 flush：托盘菜单「退出」走 `app.exit(0)`、关窗是藏到托盘，都等不到 `pagehide`，
 * 而点托盘之前窗口必然先失焦（review 第 4 条）。
 */
export function startPerf(target: Perf = perf): void {
  if (!target.enabled) return
  window.setInterval(() => target.flush(), PERF_FLUSH_MS)
  window.addEventListener('focus', () => target.mark('focus'))
  window.addEventListener('blur', () => {
    target.mark('blur')
    target.flush()
  })
  window.addEventListener('visibilitychange', () => target.flush())
  window.addEventListener('pagehide', () => target.flush())
}
