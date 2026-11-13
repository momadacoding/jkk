/**
 * 占着 worktree 的进程（后端 `worktree_holders` 的返回）。
 *
 * 删 worktree 撞上 `worktree_busy` 时，弹框据此列出进程、给出「结束进程」按钮。
 */

/** 进程是怎么跟 worktree 扯上关系的。和 Rust 侧 `HoldKind` 的 camelCase 序列化一一对应。 */
export type HoldKind = 'cwd' | 'openFile' | 'commandLine'

export interface WorktreeHolder {
  pid: number
  /** 可执行文件名，如 `node.exe`。 */
  name: string
  kind: HoldKind
  /** 被占的具体路径：工作目录、被打开的文件；命令行匹配时就是 worktree 根。 */
  path: string
  /** 本应用自己、系统关键进程为 false：列出来让人知道是谁，但不给结束。 */
  killable: boolean
  /** 本应用自己，或者它启动的子进程（WebView2、git 守护进程……）。 */
  isSelf: boolean
  /** 只有 `commandLine` 一档才有：命中路径附近的一段命令行，让用户看见依据再决定。 */
  commandLine: string | null
}

/** 删除失败的错误码是这个时，弹框才去查占用进程。 */
export const WORKTREE_BUSY = 'worktree_busy'

function norm(p: string): string {
  return p.replace(/\\/g, '/').replace(/\/+$/, '')
}

/**
 * `path` 相对 `root` 的部分；就是 root 本身时返回空串，不在 root 底下时原样返回。
 *
 * 大小写不敏感：PEB 里读出来的工作目录盘符、目录名大小写不保证和 git 给的一致。
 */
export function relativeToWorktree(path: string, root: string): string {
  const p = norm(path)
  const r = norm(root)
  if (!r) return p
  const lp = p.toLowerCase()
  const lr = r.toLowerCase()
  if (lp === lr) return ''
  if (lp.startsWith(`${lr}/`)) return p.slice(r.length + 1)
  return p
}

/** 一行说明：这个进程是怎么占着 worktree 的。 */
export function holderReason(holder: WorktreeHolder, worktreePath: string): string {
  const rel = relativeToWorktree(holder.path, worktreePath)
  switch (holder.kind) {
    case 'cwd':
      return rel ? `工作目录停在 ${rel}` : '工作目录停在 worktree 根目录'
    case 'openFile':
      return rel ? `打开着 ${rel}` : '打开着 worktree 里的文件'
    case 'commandLine':
      // 只是旁证：参数里写着这个路径，不一定是它挡住了删除
      return '启动参数指向这个 worktree（可能是它，也可能无关）'
  }
}

/** 不给结束时的原因，显示在灰掉的按钮旁边。 */
export function notKillableReason(holder: WorktreeHolder): string {
  if (!holder.isSelf) return '系统关键进程，不能从这里结束'
  // 工作目录停在里面的「自己」多半是：本应用是从停在这个 worktree 里的终端启动的，
  // 它的子进程继承了那个工作目录。关标签页改变不了工作目录
  return holder.kind === 'cwd'
    ? '是本应用或它的子进程：从别处重新启动本应用再试'
    : '是本应用自己：关掉打开着这个 worktree 的标签页再试'
}
