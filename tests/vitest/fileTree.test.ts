import { describe, expect, it } from 'vitest'
import { buildFileTree, collectDirPaths, treeFilePaths, type TreeDir, type TreeNode } from '../../src/engine/fileTree'
import type { FileStatus } from '../../src/engine/types'

const entry = (path: string, status: FileStatus | null = 'modified') => ({ path, status })

/** 把树压成 `dir/` 与 `file` 的缩进文本，断言起来比嵌套对象直观得多。 */
function render(nodes: TreeNode[], depth = 0): string[] {
  return nodes.flatMap((node) => {
    const indent = '  '.repeat(depth)
    if (node.kind === 'file') return [`${indent}${node.name}`]
    return [`${indent}${node.name}/`, ...render(node.children, depth + 1)]
  })
}

const dir = (nodes: TreeNode[], name: string): TreeDir => {
  const found = nodes.find((n) => n.kind === 'dir' && n.name === name)
  if (!found || found.kind !== 'dir') throw new Error(`没有目录 ${name}`)
  return found
}

describe('buildFileTree', () => {
  it('按路径分段建出嵌套目录', () => {
    const tree = buildFileTree([
      entry('src/components/RepoTab.tsx'),
      entry('src/engine/types.ts'),
      entry('README.md'),
    ])
    expect(render(tree)).toEqual([
      'src/',
      '  components/',
      '    RepoTab.tsx',
      '  engine/',
      '    types.ts',
      'README.md',
    ])
  })

  it('目录排在文件前面，同类按名字排', () => {
    const tree = buildFileTree([entry('z.ts'), entry('a.ts'), entry('b/x.ts'), entry('a/y.ts')])
    expect(render(tree)).toEqual(['a/', '  y.ts', 'b/', '  x.ts', 'a.ts', 'z.ts'])
  })

  // 本地改动的 Unstaged 区（第 85 条）：merge / rebase 停下来后冲突是唯一必须先处理的东西，
  // 树形显示后不能被埋进按字母排在后面的目录里
  it('冲突文件和含冲突的目录排在同层最前面，其余照旧', () => {
    const tree = buildFileTree([
      entry('a/plain.ts'),
      entry('z/deep/c.ts', 'conflicted'),
      entry('z/deep/b.ts'),
      entry('root.ts', 'conflicted'),
      entry('b.ts'),
    ])
    expect(render(tree)).toEqual([
      'z/',
      '  deep/',
      '    c.ts',
      '    b.ts',
      'root.ts',
      'a/',
      '  plain.ts',
      'b.ts',
    ])
  })

  it('同名目录只建一次，文件都挂在同一个节点下', () => {
    const tree = buildFileTree([entry('src/a.ts'), entry('src/b.ts'), entry('src/c.ts')])
    expect(tree).toHaveLength(1)
    expect(dir(tree, 'src').children).toHaveLength(3)
  })

  it('目录路径是从根算起的完整前缀', () => {
    const tree = buildFileTree([entry('src/engine/types.ts')])
    const engine = dir(dir(tree, 'src').children, 'engine')
    expect(engine.path).toBe('src/engine')
    expect(engine.children[0].path).toBe('src/engine/types.ts')
  })

  it('路径重复时只留一个节点，状态取最后一条', () => {
    const tree = buildFileTree([entry('a.ts', 'added'), entry('a.ts', 'modified')])
    expect(tree).toHaveLength(1)
    expect(tree[0].kind === 'file' && tree[0].status).toBe('modified')
  })

  it('把反斜杠当成分隔符，Windows 路径不会拼成一个文件名', () => {
    const tree = buildFileTree([entry('src\\engine\\types.ts')])
    expect(render(tree)).toEqual(['src/', '  engine/', '    types.ts'])
  })

  it('忽略空路径和多余的斜杠', () => {
    const tree = buildFileTree([entry(''), entry('/src//a.ts')])
    expect(render(tree)).toEqual(['src/', '  a.ts'])
    expect(tree[0].path).toBe('src')
  })

  it('没有输入就是空树', () => {
    expect(buildFileTree([])).toEqual([])
  })
})

describe('collectDirPaths', () => {
  it('返回每一层目录，文件不算', () => {
    const tree = buildFileTree([entry('src/engine/types.ts'), entry('tests/a.ts'), entry('README.md')])
    expect(collectDirPaths(tree).sort()).toEqual(['src', 'src/engine', 'tests'])
  })
})

describe('treeFilePaths', () => {
  const tree = buildFileTree([entry('src/b.ts'), entry('src/a/x.ts'), entry('README.md')])

  it('按树的显示顺序列出全部文件，目录不算', () => {
    expect(treeFilePaths(tree)).toEqual(['src/a/x.ts', 'src/b.ts', 'README.md'])
  })

  it('传了展开判断时跳过收起目录里的文件（Shift / 拖动选一段只选看得见的）', () => {
    expect(treeFilePaths(tree, (dir) => dir !== 'src/a')).toEqual(['src/b.ts', 'README.md'])
    expect(treeFilePaths(tree, (dir) => dir !== 'src')).toEqual(['README.md'])
  })

  it('空树是空列表', () => {
    expect(treeFilePaths([])).toEqual([])
  })
})
