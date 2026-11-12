import { readFileSync, readdirSync, statSync } from 'node:fs'
import path from 'node:path'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { PERF_FLUSH_MS, createPerf, perf, perfMark, perfSpan, type PerfDeps } from '@/store/perf'
import { createInvoke, invoke } from '@/store/ipc'
import { invoke as tauriInvoke } from '@tauri-apps/api/core'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

function setup(enabled: boolean) {
  let t = 1000
  const sent: string[][] = []
  const deps: PerfDeps = {
    enabled,
    wall: () => t,
    mono: () => t,
    send: (lines) => sent.push(lines),
  }
  const p = createPerf(deps)
  const advance = (ms: number) => {
    t += ms
  }
  const records = () => {
    p.flush()
    return sent.flat().map((l) => JSON.parse(l))
  }
  return { p, advance, records, sent }
}

afterEach(() => {
  vi.mocked(tauriInvoke).mockReset()
  vi.useRealTimers()
})

describe('默认实例（没有注入开关）', () => {
  it('测试环境里是关闭的：打点直接调原函数，invoke 不发 perf_append', async () => {
    expect(perf.enabled).toBe(false)
    const fn = vi.fn(async () => 42)
    await expect(perfSpan('x', fn)).resolves.toBe(42)
    perfMark('y')
    perf.flush()
    expect(fn).toHaveBeenCalledTimes(1)
    expect(tauriInvoke).not.toHaveBeenCalled()
  })

  it('invoke 原样转发：参数个数不变，返回的就是底层的 Promise', () => {
    const raw = Promise.resolve('ok')
    vi.mocked(tauriInvoke).mockReturnValue(raw)
    expect(invoke('load_session')).toBe(raw)
    expect(vi.mocked(tauriInvoke).mock.calls[0]).toEqual(['load_session'])
    invoke('read_status', { path: 'p' })
    expect(vi.mocked(tauriInvoke).mock.calls[1]).toEqual(['read_status', { path: 'p' }])
  })
})

describe('createInvoke', () => {
  it('关闭时不经过 timeInvoke', () => {
    const { p } = setup(false)
    const spy = vi.spyOn(p, 'timeInvoke')
    const raw = vi.fn(() => Promise.resolve(1)) as unknown as typeof tauriInvoke
    void createInvoke(p, raw)('cmd', { a: 1 })
    expect(spy).not.toHaveBeenCalled()
    expect(raw).toHaveBeenCalledWith('cmd', { a: 1 })
  })

  it('打开时记一条 invoke，结果原样交回', async () => {
    const { p, advance, records } = setup(true)
    const raw = vi.fn(async () => {
      advance(25)
      return { staged: [1, 2, 3], unstaged: [] }
    }) as unknown as typeof tauriInvoke
    const result = await createInvoke(p, raw)('read_status', { path: 'C:\\secret' })
    expect(result).toEqual({ staged: [1, 2, 3], unstaged: [] })
    const [rec] = records()
    expect(rec).toEqual({
      ts: 1000,
      dur: 25,
      src: 'web',
      kind: 'invoke',
      name: 'read_status',
      ok: true,
      meta: { 'out.staged': 3, 'out.unstaged': 0 },
    })
    expect(JSON.stringify(rec)).not.toContain('secret')
  })

  it('打开时异常原样抛出，记录 ok 为 false', async () => {
    const { p, records } = setup(true)
    const err = { code: 'git_cli', message: 'boom' }
    const raw = vi.fn(() => Promise.reject(err)) as unknown as typeof tauriInvoke
    await expect(createInvoke(p, raw)('commit', { path: 'x', message: 'secret msg' })).rejects.toBe(err)
    const [rec] = records()
    expect(rec).toMatchObject({ kind: 'invoke', name: 'commit', ok: false })
    expect(JSON.stringify(rec)).not.toContain('secret')
  })
})

describe('createPerf', () => {
  it('span 计耗时，结果原样交回；抛错时照样抛、记 ok false', async () => {
    const { p, advance, records } = setup(true)
    await expect(
      p.span('commit', async () => {
        advance(40)
        return 'done'
      }, { staged: 3 }),
    ).resolves.toBe('done')
    const boom = new Error('boom')
    await expect(p.span('commit.invoke', () => Promise.reject(boom))).rejects.toBe(boom)
    const recs = records()
    expect(recs[0]).toEqual({ ts: 1000, dur: 40, src: 'web', kind: 'span', name: 'commit', ok: true, meta: { staged: 3 } })
    expect(recs[1]).toMatchObject({ kind: 'span', name: 'commit.invoke', ok: false })
  })

  it('createInvoke 按调用时的开关走：中途 disable 之后原样转发', () => {
    const { p } = setup(true)
    const raw = vi.fn(() => Promise.resolve(1)) as unknown as typeof tauriInvoke
    const spy = vi.spyOn(p, 'timeInvoke')
    const inv = createInvoke(p, raw)
    void inv('a')
    p.disable()
    void inv('b')
    expect(spy).toHaveBeenCalledTimes(1)
  })

  it('关闭时 span 不计时也不记录，返回的就是原函数的 Promise', async () => {
    const { p, records, sent } = setup(false)
    const inner = Promise.resolve(1)
    expect(p.span('x', () => inner)).toBe(inner)
    await p.span('x', async () => 1)
    p.mark('y')
    expect(records()).toEqual([])
    expect(sent).toEqual([])
  })

  it('okOf 按返回值判成败：handleCommit 失败时返回 false 而不抛', async () => {
    const { p, records } = setup(true)
    await p.span('commit', async () => false, undefined, (r) => r)
    await p.span('commit', async () => true, undefined, (r) => r)
    expect(records().map((r) => r.ok)).toEqual([false, true])
  })

  it('ts 取墙钟、dur 取单调时钟，两者互不影响', async () => {
    let wall = 5_000
    let mono = 10
    const sent: string[][] = []
    const p = createPerf({ enabled: true, wall: () => wall, mono: () => mono, send: (l) => sent.push(l) })
    await p.span('x', async () => {
      wall += 60_000 // 中途校时 / 休眠唤醒，墙钟跳了一分钟
      mono += 7
    })
    p.flush()
    expect(JSON.parse(sent[0][0])).toMatchObject({ ts: 5_000, dur: 7 })
  })

  it('disable 之后按关闭处理，缓冲里没发的丢掉', async () => {
    const { p, sent } = setup(true)
    p.mark('a')
    p.disable()
    expect(p.enabled).toBe(false)
    p.mark('b')
    const inner = Promise.resolve(1)
    expect(p.span('x', () => inner)).toBe(inner)
    p.flush()
    expect(sent).toEqual([])
  })

  it('mark 记瞬时事件', () => {
    const { p, records } = setup(true)
    p.mark('repo-changed')
    expect(records()).toEqual([{ ts: 1000, dur: 0, src: 'web', kind: 'mark', name: 'repo-changed', ok: true }])
  })

  it('满 100 条立刻发、不等定时器', () => {
    const { p, sent } = setup(true)
    for (let i = 0; i < 100; i++) p.mark('m')
    expect(sent).toHaveLength(1)
    expect(sent[0]).toHaveLength(100)
  })
})

describe('startPerf', () => {
  it(`打开时每 ${PERF_FLUSH_MS} ms flush 一次，焦点切换各记一条 mark`, async () => {
    vi.useFakeTimers()
    const listeners = new Map<string, () => void>()
    const fakeWindow = {
      setInterval: (fn: () => void, ms: number) => setInterval(fn, ms),
      addEventListener: (type: string, fn: () => void) => listeners.set(type, fn),
    }
    vi.stubGlobal('window', fakeWindow)
    try {
      const { startPerf } = await import('@/store/perf')
      const { p, sent } = setup(true)
      startPerf(p)
      listeners.get('focus')!()
      p.mark('commit-ish')
      expect(sent).toHaveLength(0)
      vi.advanceTimersByTime(PERF_FLUSH_MS)
      expect(sent).toHaveLength(1)
      expect(sent[0].map((l) => JSON.parse(l).name)).toEqual(['focus', 'commit-ish'])
      vi.advanceTimersByTime(PERF_FLUSH_MS)
      expect(sent).toHaveLength(1)
      // 失焦立刻发：托盘「退出」等不到下一次定时器
      p.mark('late')
      listeners.get('blur')!()
      expect(sent).toHaveLength(2)
      expect(sent[1].map((l) => JSON.parse(l).name)).toEqual(['late', 'blur'])
      p.mark('later')
      listeners.get('visibilitychange')!()
      expect(sent).toHaveLength(3)
      p.mark('last')
      listeners.get('pagehide')!()
      expect(sent).toHaveLength(4)
    } finally {
      vi.unstubAllGlobals()
    }
  })

  it('关闭时什么都不装', async () => {
    const addEventListener = vi.fn()
    const setIntervalSpy = vi.fn()
    vi.stubGlobal('window', { addEventListener, setInterval: setIntervalSpy })
    try {
      const { startPerf } = await import('@/store/perf')
      startPerf(setup(false).p)
      expect(addEventListener).not.toHaveBeenCalled()
      expect(setIntervalSpy).not.toHaveBeenCalled()
    } finally {
      vi.unstubAllGlobals()
    }
  })
})

/** 漏一个直接 import 的文件，记录里就少了它发的命令（设计 §8 最后一行：`commit` 就在 RepoTab 里）。 */
describe('invoke 出口', () => {
  it('src/ 下只有 ipc.ts 与 perf.ts 直接从 @tauri-apps/api/core 取 invoke', () => {
    const root = path.resolve(__dirname, '../../src')
    const offenders: string[] = []
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const full = path.join(dir, name)
        if (statSync(full).isDirectory()) walk(full)
        else if (/\.tsx?$/.test(name)) {
          const text = readFileSync(full, 'utf8')
          if (/import\s*\{[^}]*\binvoke\b[^}]*\}\s*from\s*'@tauri-apps\/api\/core'/.test(text)) {
            offenders.push(path.relative(root, full))
          }
        }
      }
    }
    walk(root)
    expect(offenders.sort()).toEqual([path.join('store', 'ipc.ts'), path.join('store', 'perf.ts')].sort())
  })
})
