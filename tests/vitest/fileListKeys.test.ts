import { describe, expect, it } from 'vitest'
import { discardShortcut, isSelectAllShortcut, type FileListKeyEvent } from '@/engine/fileListKeys'
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

describe('isSelectAllShortcut', () => {
  const a: FileListKeyEvent = { key: 'a', ctrlKey: true, metaKey: false, shiftKey: false, altKey: false }

  it('Ctrl+A 和 macOS 的 Cmd+A 都算；开着大写锁定时 key 是 A 也算', () => {
    expect(isSelectAllShortcut(a)).toBe(true)
    expect(isSelectAllShortcut({ ...a, ctrlKey: false, metaKey: true })).toBe(true)
    expect(isSelectAllShortcut({ ...a, key: 'A' })).toBe(true)
  })

  it('俄文布局下 key 是「ф」：按物理键位 KeyA 认出来', () => {
    expect(isSelectAllShortcut({ ...a, key: 'ф', code: 'KeyA' })).toBe(true)
  })

  it('AZERTY 上 KeyA 那个键是 Q：Ctrl+Q 不算全选', () => {
    expect(isSelectAllShortcut({ ...a, key: 'q', code: 'KeyA' })).toBe(false)
  })

  it('按住不放的自动重复也算：全选是幂等的，重复无害', () => {
    expect(isSelectAllShortcut({ ...a, repeat: true })).toBe(true)
  })

  it('不带 Ctrl/Cmd、带了 Shift 或 Alt、输入法组词中都不算', () => {
    expect(isSelectAllShortcut({ ...a, ctrlKey: false })).toBe(false)
    expect(isSelectAllShortcut({ ...a, shiftKey: true })).toBe(false)
    expect(isSelectAllShortcut({ ...a, altKey: true })).toBe(false)
    expect(isSelectAllShortcut({ ...a, isComposing: true })).toBe(false)
    expect(isSelectAllShortcut({ ...a, key: 'b' })).toBe(false)
  })
})
