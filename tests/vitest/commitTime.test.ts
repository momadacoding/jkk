// 显示哪个时间（2026-11-26 报告）：跟进重放过的提交，提交时间是跟进那一刻，作者时间才是写下的时间。
import { describe, expect, it } from 'vitest'
import { rewrittenTime, rewrittenTitle, shownTime } from '@/engine/commitTime'

const WRITTEN = 1_794_192_844 // 2026-11-09 10:54:04 +08:00
const REPLAYED = 1_795_663_503 // 2026-11-26 11:25:03 +08:00

describe('shownTime', () => {
  it('有作者时间时显示作者时间', () => {
    expect(shownTime({ time: REPLAYED, authorTime: WRITTEN })).toBe(WRITTEN)
  })

  it('没有作者时间时退回 time（两个时间相同的提交、stash、svn 节点）', () => {
    expect(shownTime({ time: REPLAYED })).toBe(REPLAYED)
  })
})

describe('rewrittenTime', () => {
  it('两个时间不同时给出提交时间', () => {
    expect(rewrittenTime({ time: REPLAYED, authorTime: WRITTEN })).toBe(REPLAYED)
  })

  it('没有作者时间时没有第二个时间可说', () => {
    expect(rewrittenTime({ time: REPLAYED })).toBeNull()
  })

  it('作者时间等于提交时间（前端自己拼的节点可能带上）时同样不说', () => {
    expect(rewrittenTime({ time: REPLAYED, authorTime: REPLAYED })).toBeNull()
  })
})

describe('rewrittenTitle', () => {
  it('重放过：说明显示的是作者时间，并给出提交时间', () => {
    const title = rewrittenTitle({ time: REPLAYED, authorTime: WRITTEN })
    expect(title).toContain('作者时间')
    expect(title).toContain(`提交时间：${new Date(REPLAYED * 1000).toLocaleString()}`)
  })

  it('没重放过（含 stash 这类只有一个时间的节点）：没有提示，不硬说是作者时间', () => {
    expect(rewrittenTitle({ time: REPLAYED })).toBeUndefined()
  })
})
