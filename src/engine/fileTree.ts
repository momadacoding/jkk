import type { FileStatus } from './types'

/** 建树的输入：一条扁平路径 + 它的状态。 */
export interface TreeEntry {
  path: string
  /** null = 没有变更（File Tree 视图里的普通文件），不画状态角标。 */
  status: FileStatus | null
}

export interface TreeFile {
  kind: 'file'
  /** 该层显示的名字，即路径最后一段。 */
  name: string
  /** 完整路径，同时用作选中标识和 React key。 */
  path: string
  status: FileStatus | null
}

export interface TreeDir {
  kind: 'dir'
  name: string
  /** 从根算起的目录路径，如 `src/components`，用作展开状态的 key。 */
  path: string
  children: TreeNode[]
}

export type TreeNode = TreeDir | TreeFile

/** 目录排在文件前面，同类之间按名字排。和 Fork / VS Code 的资源树一致。 */
function compare(a: TreeNode, b: TreeNode): number {
  if (a.kind !== b.kind) return a.kind === 'dir' ? -1 : 1
  return a.name.localeCompare(b.name)
}

/**
 * 递归排序，返回这一层里有没有冲突文件。
 *
 * 冲突文件、以及含冲突的目录排在同层最前面（第 85 条）：merge / rebase 停下来之后
 * 它们是唯一必须先处理的东西，平铺列表时代就是冲突置顶，树形显示后不能被埋进
 * 按字母排在后面的目录里。提交详情的树没有 `conflicted` 状态，排序不受影响。
 */
function sortDeep(nodes: TreeNode[]): boolean {
  const hasConflict = new Map<TreeNode, boolean>()
  for (const node of nodes) {
    hasConflict.set(node, node.kind === 'dir' ? sortDeep(node.children) : node.status === 'conflicted')
  }
  const rank = (node: TreeNode) => (hasConflict.get(node) ? 0 : 1)
  nodes.sort((a, b) => rank(a) - rank(b) || compare(a, b))
  return nodes.some((node) => hasConflict.get(node))
}

/**
 * 把扁平路径列表建成目录树。
 *
 * 不做「单子目录折叠」（`a/b/c` 不会显示成一行 `a/b/c`）——Fork 也不折叠，
 * 折叠之后目录的展开状态和路径前缀就对不上了。
 *
 * 同一路径出现多次时以**最后一条**为准：工作区 diff 里同一个文件可能既在
 * staged 又在 unstaged 出现，重复建两个节点会让树里凭空多出一行。
 */
export function buildFileTree(entries: TreeEntry[]): TreeNode[] {
  const roots: TreeNode[] = []
  /** 目录路径 → 该目录节点，避免每加一个文件都从根重新查一遍。 */
  const dirs = new Map<string, TreeDir>()
  /** 文件路径 → 已建好的文件节点，用于去重。 */
  const files = new Map<string, TreeFile>()

  for (const entry of entries) {
    // git 给的是正斜杠，但工作区路径在 Windows 上可能混进反斜杠
    const segments = entry.path.replace(/\\/g, '/').split('/').filter(Boolean)
    if (segments.length === 0) continue

    const name = segments[segments.length - 1]
    const full = segments.join('/')

    const existing = files.get(full)
    if (existing) {
      existing.status = entry.status
      continue
    }

    let siblings = roots
    let prefix = ''
    for (const segment of segments.slice(0, -1)) {
      prefix = prefix ? `${prefix}/${segment}` : segment
      let dir = dirs.get(prefix)
      if (!dir) {
        dir = { kind: 'dir', name: segment, path: prefix, children: [] }
        dirs.set(prefix, dir)
        siblings.push(dir)
      }
      siblings = dir.children
    }

    const file: TreeFile = { kind: 'file', name, path: full, status: entry.status }
    files.set(full, file)
    siblings.push(file)
  }

  sortDeep(roots)
  return roots
}

/**
 * 树里的文件路径，按显示顺序（深度优先，目录在前）。
 *
 * 本地改动的选中、Shift 范围、默认显示哪一个都按「眼睛看到的顺序」算，
 * 这里是那份顺序的唯一来源。
 *
 * @param isOpen 传了就跳过收起目录里的文件——Shift / 拖动选一段只该选到看得见的行，
 *   否则收起的目录被整段夹进选中，接着按 Delete 就会丢掉看不见的文件
 */
export function treeFilePaths(nodes: TreeNode[], isOpen?: (dirPath: string) => boolean): string[] {
  const out: string[] = []
  const walk = (list: TreeNode[]) => {
    for (const node of list) {
      if (node.kind === 'file') out.push(node.path)
      else if (!isOpen || isOpen(node.path)) walk(node.children)
    }
  }
  walk(nodes)
  return out
}

/** 树里所有目录的路径。用来把树默认全展开。 */
export function collectDirPaths(nodes: TreeNode[]): string[] {
  const out: string[] = []
  const walk = (list: TreeNode[]) => {
    for (const node of list) {
      if (node.kind !== 'dir') continue
      out.push(node.path)
      walk(node.children)
    }
  }
  walk(nodes)
  return out
}
