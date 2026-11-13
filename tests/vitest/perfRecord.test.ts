import { describe, expect, it, vi } from 'vitest'
import {
  PERF_BATCH_MAX,
  PerfBuffer,
  arrayCounts,
  invokeRecord,
  markRecord,
  spanRecord,
} from '@/engine/perfRecord'

describe('arrayCounts', () => {
  it('只取顶层数组字段的长度，不带元素内容', () => {
    const meta = arrayCounts(
      { path: 'C:\\secret\\repo', paths: ['a.txt', 'b.txt', 'c.txt'], message: 'top secret', nested: { files: [1] } },
      'in.',
    )
    expect(meta).toEqual({ 'in.paths': 3 })
    expect(JSON.stringify(meta)).not.toMatch(/secret|a\.txt/)
  })

  it('值本身是数组时记 length', () => {
    expect(arrayCounts([1, 2], 'out.')).toEqual({ 'out.length': 2 })
    expect(arrayCounts([], 'out.')).toEqual({ 'out.length': 0 })
  })

  it('空值、标量、没有数组字段时为空', () => {
    for (const v of [undefined, null, 0, 'text', true, {}, { a: 1 }]) {
      expect(arrayCounts(v, 'in.')).toEqual({})
    }
  })
})

describe('记录构造', () => {
  it('invoke 记录合并参数和返回值的数量', () => {
    const rec = invokeRecord('read_status', 10, 5, true, { path: 'x', paths: ['a'] }, { staged: [1, 2], unstaged: [] })
    expect(rec).toEqual({
      ts: 10,
      dur: 5,
      src: 'web',
      kind: 'invoke',
      name: 'read_status',
      ok: true,
      meta: { 'in.paths': 1, 'out.staged': 2, 'out.unstaged': 0 },
    })
  })

  it('失败的 invoke 不看返回值，没有数量时不带 meta 键', () => {
    const rec = invokeRecord('commit', 1, 2, false, { path: 'x', message: 'm' }, { files: [1] })
    expect(rec).toEqual({ ts: 1, dur: 2, src: 'web', kind: 'invoke', name: 'commit', ok: false })
    expect('meta' in rec).toBe(false)
  })

  it('span 与 mark', () => {
    expect(spanRecord('commit.invoke', 1, 2, true, { amend: false })).toEqual({
      ts: 1, dur: 2, src: 'web', kind: 'span', name: 'commit.invoke', ok: true, meta: { amend: false },
    })
    expect(markRecord('focus', 7)).toEqual({ ts: 7, dur: 0, src: 'web', kind: 'mark', name: 'focus', ok: true })
  })
})

describe('PerfBuffer', () => {
  it(`攒满 ${PERF_BATCH_MAX} 条立刻发一批，之后从空开始`, () => {
    const send = vi.fn()
    const buf = new PerfBuffer(send)
    for (let i = 0; i < PERF_BATCH_MAX - 1; i++) buf.push(markRecord(`m${i}`, i))
    expect(send).not.toHaveBeenCalled()
    buf.push(markRecord('last', 0))
    expect(send).toHaveBeenCalledTimes(1)
    const batch = send.mock.calls[0][0] as string[]
    expect(batch).toHaveLength(PERF_BATCH_MAX)
    expect(JSON.parse(batch[0])).toMatchObject({ name: 'm0', src: 'web' })
    expect(buf.size).toBe(0)
  })

  it('flush 发剩下的；空的时候不发', () => {
    const send = vi.fn()
    const buf = new PerfBuffer(send)
    buf.flush()
    expect(send).not.toHaveBeenCalled()
    buf.push(markRecord('a', 0))
    buf.flush()
    expect(send).toHaveBeenCalledWith([expect.stringContaining('"name":"a"')])
    buf.flush()
    expect(send).toHaveBeenCalledTimes(1)
  })

  it('发送抛错时缓冲已经清空，不会越攒越多', () => {
    const buf = new PerfBuffer(() => {
      throw new Error('ipc down')
    })
    buf.push(markRecord('a', 0))
    expect(() => buf.flush()).toThrow('ipc down')
    expect(buf.size).toBe(0)
  })
})
