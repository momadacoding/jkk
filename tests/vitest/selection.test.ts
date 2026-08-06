import { describe, expect, it } from 'vitest'
import { nextSelection } from '../../src/engine/selection'

const ids = ['c3', 'c2', 'c1']

describe('nextSelection', () => {
  it('已经有选中时，普通刷新不动它', () => {
    // 后台文件监听每次都会刷图，不能把用户正在看的提交拽走
    expect(nextSelection('c2', ids)).toBe('c2')
    expect(nextSelection('c2', ids, 'c3')).toBe('c2')
  })

  it('还没有选中时用 preferred', () => {
    expect(nextSelection(null, ids, 'c2')).toBe('c2')
  })

  it('既没有选中也没有 preferred 时退回第一条', () => {
    expect(nextSelection(null, ids)).toBe('c3')
  })

  it('force 时 preferred 压过已有选中', () => {
    // pull / checkout 这类显式操作移动了 HEAD，选中要跟过去
    expect(nextSelection('c1', ids, 'c3', true)).toBe('c3')
  })

  it('force 但 preferred 不在图里时保持原选中，不留悬空 id', () => {
    // 历史有 5000 条上限，新 HEAD 可能被截断在外。选一个图里没有的 id
    // 会让详情面板空白且毫无解释——宁可不动。
    expect(nextSelection('c1', ids, 'unknown', true)).toBe('c1')
  })

  it('force 且原本就没有选中、preferred 也不在图里时退回第一条', () => {
    expect(nextSelection(null, ids, 'unknown', true)).toBe('c3')
  })

  it('空仓库返回 null', () => {
    expect(nextSelection(null, [])).toBeNull()
    expect(nextSelection(null, [], 'c1', true)).toBeNull()
  })
})
