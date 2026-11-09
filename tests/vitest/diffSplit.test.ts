import { describe, expect, it } from 'vitest'
import { contentSegments, toSplitRows } from '@/engine/diffSplit'
import type { DiffLine } from '@/engine/types'

const ctx = (content: string, o: number, n: number): DiffLine => ({ origin: ' ', content, oldLineno: o, newLineno: n })
const del = (content: string, o: number): DiffLine => ({ origin: '-', content, oldLineno: o, newLineno: null })
const add = (content: string, n: number): DiffLine => ({ origin: '+', content, oldLineno: null, newLineno: n })

describe('toSplitRows', () => {
  it('上下文两边各一份，删除与新增按顺序一一对上，多出来的一侧另一边留空', () => {
    const c1 = ctx('a', 1, 1)
    const d1 = del('b', 2)
    const d2 = del('c', 3)
    const a1 = add('B', 2)
    const c2 = ctx('d', 4, 3)
    const a2 = add('e', 4)
    expect(toSplitRows([c1, d1, d2, a1, c2, a2])).toEqual([
      { left: c1, right: c1 },
      { left: d1, right: a1 },
      { left: d2, right: null },
      { left: c2, right: c2 },
      { left: null, right: a2 },
    ])
  })

  it('空输入', () => {
    expect(toSplitRows([])).toEqual([])
  })
})

describe('contentSegments', () => {
  it('关着时只去掉行尾的 \\r', () => {
    expect(contentSegments('a b\r', false)).toEqual([{ text: 'a b', mark: null }])
    expect(contentSegments('a\rb', false)).toEqual([{ text: 'a\rb', mark: null }])
  })

  it('开着时空格、制表符、行尾 \\r 各自成段', () => {
    expect(contentSegments('\tx  y\r', true)).toEqual([
      { text: '\t', mark: 'tab' },
      { text: 'x', mark: null },
      { text: ' ', mark: 'space' },
      { text: ' ', mark: 'space' },
      { text: 'y', mark: null },
      { text: '', mark: 'cr' },
    ])
  })

  it('空行', () => {
    expect(contentSegments('', true)).toEqual([])
    expect(contentSegments('', false)).toEqual([{ text: '', mark: null }])
  })
})
