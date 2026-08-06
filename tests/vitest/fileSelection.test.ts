import { describe, expect, it } from 'vitest'
import { nextFileSelection, type FileSelection } from '../../src/engine/fileSelection'

const files = ['a.ts', 'b.ts', 'c.ts', 'd.ts', 'e.ts']
const empty: FileSelection = { section: null, paths: [], anchor: null }

const plain = { ctrl: false, shift: false }
const ctrl = { ctrl: true, shift: false }
const shift = { ctrl: false, shift: true }

describe('nextFileSelection', () => {
  it('普通单击只选中这一个', () => {
    const next = nextFileSelection(empty, 'unstaged', files, 'c.ts', plain)
    expect(next.paths).toEqual(['c.ts'])
    expect(next.section).toBe('unstaged')
    expect(next.anchor).toBe('c.ts')
  })

  it('普通单击会丢掉之前的多选', () => {
    const prev: FileSelection = { section: 'unstaged', paths: ['a.ts', 'b.ts'], anchor: 'a.ts' }
    expect(nextFileSelection(prev, 'unstaged', files, 'd.ts', plain).paths).toEqual(['d.ts'])
  })

  it('Ctrl+单击往选中里加一个', () => {
    const prev: FileSelection = { section: 'unstaged', paths: ['a.ts'], anchor: 'a.ts' }
    expect(nextFileSelection(prev, 'unstaged', files, 'c.ts', ctrl).paths).toEqual(['a.ts', 'c.ts'])
  })

  it('Ctrl+单击已选中的项会把它去掉', () => {
    const prev: FileSelection = { section: 'unstaged', paths: ['a.ts', 'c.ts'], anchor: 'a.ts' }
    expect(nextFileSelection(prev, 'unstaged', files, 'a.ts', ctrl).paths).toEqual(['c.ts'])
  })

  it('Shift+单击选中从锚点到这一项的整段', () => {
    const prev: FileSelection = { section: 'unstaged', paths: ['b.ts'], anchor: 'b.ts' }
    expect(nextFileSelection(prev, 'unstaged', files, 'd.ts', shift).paths).toEqual(['b.ts', 'c.ts', 'd.ts'])
  })

  it('Shift 往回选也要按列表顺序返回', () => {
    // 顺序错了的话，后续按这个顺序做的操作和界面显示对不上
    const prev: FileSelection = { section: 'unstaged', paths: ['d.ts'], anchor: 'd.ts' }
    expect(nextFileSelection(prev, 'unstaged', files, 'b.ts', shift).paths).toEqual(['b.ts', 'c.ts', 'd.ts'])
  })

  it('Shift 不移动锚点，可以连续调整范围', () => {
    const prev: FileSelection = { section: 'unstaged', paths: ['b.ts'], anchor: 'b.ts' }
    const once = nextFileSelection(prev, 'unstaged', files, 'd.ts', shift)
    expect(once.anchor).toBe('b.ts')
    const twice = nextFileSelection(once, 'unstaged', files, 'c.ts', shift)
    expect(twice.paths).toEqual(['b.ts', 'c.ts'])
  })

  it('没有锚点时 Shift 退化成单选', () => {
    expect(nextFileSelection(empty, 'unstaged', files, 'c.ts', shift).paths).toEqual(['c.ts'])
  })

  it('换到另一个区就清空原来的选中', () => {
    // Unstaged 和 Staged 的操作语义不同，跨区多选做出来的动作没法解释
    const prev: FileSelection = { section: 'unstaged', paths: ['a.ts', 'b.ts'], anchor: 'a.ts' }
    const next = nextFileSelection(prev, 'staged', ['x.ts'], 'x.ts', ctrl)
    expect(next.section).toBe('staged')
    expect(next.paths).toEqual(['x.ts'])
  })

  it('右键点在已选中的项上时保持整片选中', () => {
    const prev: FileSelection = { section: 'unstaged', paths: ['a.ts', 'b.ts'], anchor: 'a.ts' }
    expect(nextFileSelection(prev, 'unstaged', files, 'b.ts', { ctrl: false, shift: false, keepIfSelected: true }).paths)
      .toEqual(['a.ts', 'b.ts'])
  })

  it('右键点在未选中的项上时先改选它', () => {
    const prev: FileSelection = { section: 'unstaged', paths: ['a.ts', 'b.ts'], anchor: 'a.ts' }
    expect(nextFileSelection(prev, 'unstaged', files, 'd.ts', { ctrl: false, shift: false, keepIfSelected: true }).paths)
      .toEqual(['d.ts'])
  })

  it('Ctrl 把最后一项也去掉后，选中为空', () => {
    const prev: FileSelection = { section: 'unstaged', paths: ['a.ts'], anchor: 'a.ts' }
    const next = nextFileSelection(prev, 'unstaged', files, 'a.ts', ctrl)
    expect(next.paths).toEqual([])
  })
})
