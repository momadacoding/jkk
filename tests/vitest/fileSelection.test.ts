import { describe, expect, it } from 'vitest'
import {
  activeChange,
  changesPaths,
  changesSection,
  nextFileSelection,
  reconcileSelection,
  type FileSelection,
} from '../../src/engine/fileSelection'
import type { FileChange } from '../../src/engine/types'

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

// 第 65 条：右侧 diff 按「哪个区的哪个文件」读，区决定 scope。
const sel = (section: 'unstaged' | 'staged', ...paths: string[]): FileSelection =>
  ({ section, paths, anchor: paths[0] ?? null })

// 状态刷新后选中要跟着列表对齐：选中本身是真相，高亮、快捷键、读哪个区都从它来
describe('reconcileSelection', () => {
  it('选中的都还在：原样返回（同一个对象，不触发多余渲染）', () => {
    const s = sel('unstaged', 'a', 'b')
    expect(reconcileSelection(s, ['a', 'b', 'c'], [])).toBe(s)
  })

  it('还没选过：保持为空', () => {
    expect(reconcileSelection(empty, ['a'], ['b'])).toBe(empty)
  })

  it('多选里有一项不见了：留下其余的，不跳到别的文件', () => {
    const s: FileSelection = { section: 'staged', paths: ['x', 'y'], anchor: 'y' }
    expect(reconcileSelection(s, ['u1'], ['x'])).toEqual({ section: 'staged', paths: ['x'], anchor: 'x' })
  })

  it('选中的文件刚被整份暂存：选中跟到 Staged 的同一个文件', () => {
    expect(reconcileSelection(sel('unstaged', 'a'), ['b'], ['a'])).toEqual(sel('staged', 'a'))
  })

  it('选中的文件刚被取消暂存：跟到 Unstaged', () => {
    expect(reconcileSelection(sel('staged', 'a'), ['a'], ['b'])).toEqual(sel('unstaged', 'a'))
  })

  it('多选整批暂存：整批跟到 Staged，不缩成一个（马上反悔再整批 Unstage 时还是这一批）', () => {
    const s: FileSelection = { section: 'unstaged', paths: ['a', 'b', 'c'], anchor: 'a' }
    expect(reconcileSelection(s, ['x'], ['a', 'b', 'c'])).toEqual({ section: 'staged', paths: ['a', 'b', 'c'], anchor: 'a' })
  })

  it('整批里只有一部分跑到了另一个区：跟过去的是那一部分，按那个区的显示顺序', () => {
    const s: FileSelection = { section: 'unstaged', paths: ['b', 'a', 'gone'], anchor: 'gone' }
    expect(reconcileSelection(s, ['x'], ['a', 'b'])).toEqual({ section: 'staged', paths: ['a', 'b'], anchor: 'a' })
  })

  it('选中的两边都没有了（刚提交、刚丢弃）：清空', () => {
    expect(reconcileSelection(sel('staged', 'gone'), ['u1'], ['s1'])).toEqual(empty)
  })
})

describe('activeChange', () => {
  it('还没选过：Unstaged 的第一个', () => {
    expect(activeChange(empty, ['u1', 'u2'], ['s1'])).toEqual({ path: 'u1', section: 'unstaged' })
  })

  it('还没选过、Unstaged 是空的：Staged 的第一个', () => {
    expect(activeChange(empty, [], ['s1', 's2'])).toEqual({ path: 's1', section: 'staged' })
  })

  it('两边都空：没有', () => {
    expect(activeChange(empty, [], [])).toBeNull()
  })

  it('选中 Staged 里的文件：就是它、就是 Staged', () => {
    expect(activeChange(sel('staged', 's2'), ['u1'], ['s1', 's2'])).toEqual({ path: 's2', section: 'staged' })
  })

  it('同一个文件两边都有：按选中时所在的那个区', () => {
    expect(activeChange(sel('unstaged', 'a'), ['a'], ['a'])).toEqual({ path: 'a', section: 'unstaged' })
    expect(activeChange(sel('staged', 'a'), ['a'], ['a'])).toEqual({ path: 'a', section: 'staged' })
  })

  it('多选时看最后点的那一个', () => {
    expect(activeChange(sel('unstaged', 'u1', 'u2'), ['u1', 'u2'], [])).toEqual({ path: 'u2', section: 'unstaged' })
  })

  it('先对齐再取：选中的文件刚被整份暂存，就是 Staged 里的它', () => {
    expect(activeChange(sel('unstaged', 'a'), ['b'], ['a'])).toEqual({ path: 'a', section: 'staged' })
  })

  it('先对齐再取：多选的最后一项不见了，取剩下的，不跳到 Unstaged 的第一个', () => {
    expect(activeChange(sel('staged', 'x', 'y'), ['u1'], ['x'])).toEqual({ path: 'x', section: 'staged' })
  })
})

describe('changesSection', () => {
  const ch = (path: string, status = 'modified'): FileChange => ({ path, status, additions: 0, deletions: 0 })

  it('选中 Staged 里的：staged', () => {
    expect(changesSection(sel('staged', 's'), { unstaged: [ch('u')], staged: [ch('s')] })).toBe('staged')
  })

  it('没选、只有 Staged 有文件：staged', () => {
    expect(changesSection(empty, { unstaged: [], staged: [ch('s')] })).toBe('staged')
  })

  it('两边都空：unstaged（没东西可读，取哪个都一样，固定一个免得来回切）', () => {
    expect(changesSection(empty, { unstaged: [], staged: [] })).toBe('unstaged')
  })

})

describe('changesPaths', () => {
  it('按界面上的顺序：冲突文件排在 Unstaged 最前面（RepoTab 与 LocalChangesView 共用这一份）', () => {
    const ch = (path: string, status = 'modified'): FileChange => ({ path, status, additions: 0, deletions: 0 })
    expect(changesPaths({ unstaged: [ch('plain'), ch('c', 'conflicted')], staged: [ch('s')] }))
      .toEqual({ unstaged: ['c', 'plain'], staged: ['s'] })
  })
})
