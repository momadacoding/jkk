import { describe, expect, it } from 'vitest'
import {
  activeChange,
  changesDiffScope,
  changesRows,
  changesTree,
  dirRowKey,
  isDirRowKey,
  reconcileSelection,
  rowPath,
  selectAllFiles,
  selectedFiles,
  treeRowKeys,
  type FileSelection,
} from '../../src/engine/fileSelection'
import type { FileChange, StatusData } from '../../src/engine/types'

// 文件夹行也能选中（2026-11-08 方案 A）：选中里放「行」，文件夹的行 key 是 `目录/`，
// 动作（Stage / Discard / 区头按钮 / Delete）落地前再展开成文件夹下的全部文件

const ch = (path: string, status = 'modified'): FileChange => ({ path, status, additions: 0, deletions: 0 })
const sel = (section: 'unstaged' | 'staged', ...paths: string[]): FileSelection =>
  ({ section, paths, anchor: paths[0] ?? null })
const status = (unstaged: string[], staged: string[] = []): StatusData =>
  ({ unstaged: unstaged.map((p) => ch(p)), staged: staged.map((p) => ch(p)) })

describe('文件夹的行 key', () => {
  it('末尾带斜杠，与文件路径不会撞（git 报上来的文件路径不以斜杠结尾）', () => {
    expect(dirRowKey('src/components')).toBe('src/components/')
    expect(isDirRowKey('src/components/')).toBe(true)
    expect(isDirRowKey('src/components')).toBe(false)
    expect(isDirRowKey('a.txt')).toBe(false)
  })

  it('复制路径时去掉末尾斜杠，文件原样', () => {
    expect(rowPath('src/components/')).toBe('src/components')
    expect(rowPath('src/a.ts')).toBe('src/a.ts')
  })
})

describe('treeRowKeys / changesRows', () => {
  const tree = changesTree([ch('src/b.ts'), ch('a.txt'), ch('src/lib/x.ts')])

  it('按界面上的顺序：文件夹行在它的内容前面', () => {
    expect(treeRowKeys(tree)).toEqual(['src/', 'src/lib/', 'src/lib/x.ts', 'src/b.ts', 'a.txt'])
  })

  it('收起的文件夹：它自己的行还在，里面的行不在', () => {
    expect(treeRowKeys(tree, (dir) => dir !== 'src/lib')).toEqual(['src/', 'src/lib/', 'src/b.ts', 'a.txt'])
  })

  it('changesRows 两个区各一份，不管收没收起', () => {
    expect(changesRows(status(['src/a.ts'], ['b.txt']))).toEqual({ unstaged: ['src/', 'src/a.ts'], staged: ['b.txt'] })
  })

  it('文件路径保持状态里的原样（反斜杠），文件夹行用正斜杠', () => {
    expect(changesRows(status(['src\\a.ts'])).unstaged).toEqual(['src/', 'src\\a.ts'])
  })
})

describe('selectedFiles（动作落地前把选中展开成文件）', () => {
  const st = status(['src/lib/x.ts', 'src/b.ts', 'a.txt'], ['src/b.ts'])

  it('选中文件夹：它下面的全部文件，含收起的子文件夹里的，按显示顺序', () => {
    expect(selectedFiles(sel('unstaged', 'src/'), st)).toEqual(['src/lib/x.ts', 'src/b.ts'])
  })

  it('文件夹和它里面的文件同时选中：不重复', () => {
    expect(selectedFiles(sel('unstaged', 'src/b.ts', 'src/'), st)).toEqual(['src/lib/x.ts', 'src/b.ts'])
  })

  it('只选文件：原样（按显示顺序）', () => {
    expect(selectedFiles(sel('unstaged', 'a.txt', 'src/b.ts'), st)).toEqual(['src/b.ts', 'a.txt'])
  })

  it('只展开本区的：Staged 里选中 src/，只有 Staged 里的 src/b.ts', () => {
    expect(selectedFiles(sel('staged', 'src/'), st)).toEqual(['src/b.ts'])
  })

  it('没选：空', () => {
    expect(selectedFiles({ section: null, paths: [], anchor: null }, st)).toEqual([])
  })

  it('选中的文件夹已经不在本区（刷新后整个暂存走了）：空，不能退回到别的文件', () => {
    expect(selectedFiles(sel('unstaged', 'gone/'), st)).toEqual([])
  })

  it('反斜杠路径：展开出来的是原样路径', () => {
    expect(selectedFiles(sel('unstaged', 'src/'), status(['src\\a.ts']))).toEqual(['src\\a.ts'])
  })
})

describe('选中文件夹之后的对齐 / 默认项 / diff', () => {
  it('整个文件夹刚被暂存：选中跟到 Staged 的同一个文件夹（马上反悔还能整个 Unstage）', () => {
    const rows = changesRows(status(['b.txt'], ['src/a.ts', 'src/b.ts']))
    expect(reconcileSelection(sel('unstaged', 'src/'), rows.unstaged, rows.staged)).toEqual(sel('staged', 'src/'))
  })

  it('文件夹里还有文件没暂存走：选中留在 Unstaged 的这个文件夹', () => {
    const rows = changesRows(status(['src/b.ts'], ['src/a.ts']))
    const s = sel('unstaged', 'src/')
    expect(reconcileSelection(s, rows.unstaged, rows.staged)).toBe(s)
  })

  it('没选：默认显示第一个文件，跳过排在前面的文件夹行', () => {
    const rows = changesRows(status(['src/a.ts', 'b.txt']))
    expect(activeChange({ section: null, paths: [], anchor: null }, rows.unstaged, rows.staged))
      .toEqual({ path: 'src/a.ts', section: 'unstaged' })
  })

  it('最后点的是文件夹：active 就是这个文件夹（高亮它，右侧不读 diff）', () => {
    const rows = changesRows(status(['src/a.ts', 'b.txt']))
    expect(activeChange(sel('unstaged', 'src/'), rows.unstaged, rows.staged))
      .toEqual({ path: 'src/', section: 'unstaged' })
    expect(changesDiffScope(sel('unstaged', 'src/'), status(['src/a.ts', 'b.txt']))).toBeNull()
  })

  it('选中文件时照旧读它的 diff', () => {
    expect(changesDiffScope(sel('unstaged', 'src/a.ts'), status(['src/a.ts']))).toBe('unstaged:src/a.ts')
  })

  it('Ctrl+A 按看得见的行选，文件夹行也在里面', () => {
    const tree = changesTree([ch('src/a.ts'), ch('b.txt')])
    const rows = treeRowKeys(tree)
    expect(selectAllFiles({ section: null, paths: [], anchor: null }, 'unstaged', rows).paths)
      .toEqual(['src/', 'src/a.ts', 'b.txt'])
  })
})
