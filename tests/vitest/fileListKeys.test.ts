import { describe, expect, it } from 'vitest'
import { discardShortcut, type FileListKeyEvent } from '@/engine/fileListKeys'
import type { FileSelection } from '@/engine/fileSelection'

const del: FileListKeyEvent = {
  key: 'Delete',
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  altKey: false,
}

const files = {
  unstaged: ['a.txt', 'b.txt', 'c.txt'],
  staged: ['s1.txt', 's2.txt'],
}

function sel(section: FileSelection['section'], paths: string[]): FileSelection {
  return { section, paths, anchor: paths[0] ?? null }
}

describe('discardShortcut', () => {
  it('Unstaged 区选中时 Delete 丢弃选中的文件', () => {
    expect(discardShortcut(del, 'unstaged', sel('unstaged', ['a.txt', 'c.txt']), files))
      .toEqual({ paths: ['a.txt', 'c.txt'], staged: false })
  })

  it('Staged 区选中时按 staged 丢弃，和右键菜单同一语义', () => {
    expect(discardShortcut(del, 'staged', sel('staged', ['s2.txt']), files))
      .toEqual({ paths: ['s2.txt'], staged: true })
  })

  it('没有选中任何文件时什么都不做——不能退回到 diff 面板默认显示的那个文件', () => {
    expect(discardShortcut(del, 'unstaged', sel(null, []), files)).toBeNull()
    expect(discardShortcut(del, 'unstaged', sel('unstaged', []), files)).toBeNull()
  })

  it('刷新后已经不在该区的路径要剔掉，全剔光了就什么都不做', () => {
    // 典型场景：选中后在外部把文件 stage 掉了，selection 还留着旧路径
    expect(discardShortcut(del, 'unstaged', sel('unstaged', ['a.txt', 'gone.txt']), files))
      .toEqual({ paths: ['a.txt'], staged: false })
    expect(discardShortcut(del, 'unstaged', sel('unstaged', ['s1.txt']), files)).toBeNull()
  })

  it('焦点所在的区和选中所在的区不一致时不动手', () => {
    expect(discardShortcut(del, 'unstaged', sel('staged', ['s1.txt']), files)).toBeNull()
    expect(discardShortcut(del, 'staged', sel('unstaged', ['a.txt']), files)).toBeNull()
  })

  it('只认裸 Delete：其他键、带修饰键的组合都不接管', () => {
    const s = sel('unstaged', ['a.txt'])
    expect(discardShortcut({ ...del, key: 'Backspace' }, 'unstaged', s, files)).toBeNull()
    expect(discardShortcut({ ...del, key: 'Enter' }, 'unstaged', s, files)).toBeNull()
    expect(discardShortcut({ ...del, ctrlKey: true }, 'unstaged', s, files)).toBeNull()
    expect(discardShortcut({ ...del, shiftKey: true }, 'unstaged', s, files)).toBeNull()
    expect(discardShortcut({ ...del, altKey: true }, 'unstaged', s, files)).toBeNull()
    expect(discardShortcut({ ...del, metaKey: true }, 'unstaged', s, files)).toBeNull()
  })

  it('按住不放的自动重复和输入法组词中的按键都不触发', () => {
    const s = sel('unstaged', ['a.txt'])
    expect(discardShortcut({ ...del, repeat: true }, 'unstaged', s, files)).toBeNull()
    expect(discardShortcut({ ...del, isComposing: true }, 'unstaged', s, files)).toBeNull()
  })
})
