import { describe, expect, it } from 'vitest'
import { describeDiscard } from '@/engine/discardConfirm'

describe('describeDiscard', () => {
  it('能恢复的列在正文，永久删除的单独进红条', () => {
    const d = describeDiscard(['Code/A.cs', 'Code/New.cs'], ['Code/New.cs'], false)
    expect(d.message).toContain('恢复到暂存区的内容（1）')
    expect(d.message).toContain('  Code/A.cs')
    expect(d.message).not.toContain('Code/New.cs')
    expect(d.warning).toContain('永久删除，无法恢复（1）')
    expect(d.warning).toContain('  Code/New.cs')
  })

  it('打开时就看得到代价：首行写明有几个文件会永久删除', () => {
    // 能恢复的清单很长时红条在最下面，而 Discard 按钮贴底、不滚动也点得到
    const many = Array.from({ length: 300 }, (_, i) => `Code/F${i}.cs`)
    const d = describeDiscard([...many, 'Code/New.cs'], ['Code/New.cs'], false)
    const head = d.message.split('\n')[0]
    expect(head).toContain('1 个')
    expect(head).toContain('永久删除')
  })

  it('没有未跟踪文件时首行不提永久删除', () => {
    expect(describeDiscard(['Code/A.cs'], [], false).message.split('\n')[0]).not.toContain('永久删除')
  })

  it('全是能恢复的：没有红条', () => {
    expect(describeDiscard(['Code/A.cs'], [], false).warning).toBeUndefined()
  })

  it('全是未跟踪文件：正文不留空的「恢复到」段', () => {
    const d = describeDiscard(['Code/New.cs'], ['Code/New.cs'], false)
    expect(d.message).not.toContain('恢复到')
    expect(d.warning).toContain('Code/New.cs')
  })

  it('暂存区的改动恢复到 HEAD', () => {
    expect(describeDiscard(['Code/A.cs'], [], true).message).toContain('恢复到 HEAD（1）')
  })

  it('清单不截断：这是不可逆操作，用户要能看全会动到哪些文件', () => {
    const many = Array.from({ length: 500 }, (_, i) => `Code/F${i}.cs`)
    const d = describeDiscard(many, many.slice(0, 250), false)
    expect(d.warning).toContain('Code/F249.cs')
    expect(d.message).toContain('Code/F499.cs')
  })

  it('默认按钮是 Cancel：回车不能丢东西', () => {
    const d = describeDiscard(['Code/A.cs'], [], false)
    expect(d.danger).toBe(true)
    expect(d.focusCancel).toBe(true)
  })
})
