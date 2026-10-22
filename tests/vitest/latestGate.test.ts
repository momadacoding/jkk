import { describe, expect, it } from 'vitest'
import { createLatestGate } from '@/engine/latestGate'

describe('createLatestGate', () => {
  it('后发起的请求让先发起的失效', () => {
    const gate = createLatestGate()
    const first = gate.begin()
    const second = gate.begin()
    expect(gate.isLatest(first)).toBe(false)
    expect(gate.isLatest(second)).toBe(true)
  })

  it('先发起的晚到也不能覆盖：顺序只看 begin，不看完成先后', () => {
    const gate = createLatestGate()
    const slow = gate.begin('diff')
    const fast = gate.begin('diff')
    // fast 先完成、写入；slow 后完成时必须被丢掉
    expect(gate.isLatest(fast, 'diff')).toBe(true)
    expect(gate.isLatest(slow, 'diff')).toBe(false)
  })

  it('不同 key 各自计数，互不影响', () => {
    const gate = createLatestGate()
    const status = gate.begin('status')
    const diff = gate.begin('diff')
    gate.begin('diff')
    expect(gate.isLatest(status, 'status')).toBe(true)
    expect(gate.isLatest(diff, 'diff')).toBe(false)
  })

  it('两个 gate 实例互不相干', () => {
    const a = createLatestGate()
    const b = createLatestGate()
    const ta = a.begin()
    b.begin()
    b.begin()
    expect(a.isLatest(ta)).toBe(true)
  })
})
