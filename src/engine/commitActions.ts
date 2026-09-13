import type { Capabilities } from './capabilities'
import type { CommitNode } from './types'

/** graph 行右键菜单里的一项。 */
export type CommitAction =
  | 'new-branch'
  | 'new-tag'
  | 'checkout'
  | 'cherry-pick'
  | 'revert'
  | 'save-patch'
  | 'copy-hash'
  | 'copy-info'

export interface CommitMenuEntry {
  action: CommitAction
  label: string
  /** 置灰但保留位置——菜单项数量每行都一样，位置才有肌肉记忆。 */
  disabled?: boolean
  /** 非空时执行前先弹确认框，文案即此字符串。 */
  confirm?: string
}

export interface CommitMenuContext {
  /** 这条提交是不是当前 HEAD。 */
  isHead: boolean
  /**
   * 后端能力。不支持的项**整条去掉**，不是置灰。
   *
   * 与 `checkout` 那种「置灰但保留位置」的处理刻意不同：置灰表达的是
   * 「这一项此刻做不了」，而这里表达的是「这个概念在当前后端不存在」，
   * 留一个永远灰着的 `Cherry Pick` 只会让人反复去试。
   *
   * 这个字段**是必填的**。设成可选、默认全开的话，哪天新加一个调用点忘了传，
   * SVN 仓库上就会静默出现一个直连 git 的菜单项——而 `tsc` 覆盖 `src/`，
   * 必填能让它在编译期就报出来。
   */
  caps: Capabilities
}

/** 一次可以直接发出去的后端调用。 */
export interface CommitCommandPlan {
  command: string
  args: Record<string, unknown>
}

/** git 默认的 7 位缩写。短于 7 位的原样返回。 */
export function shortHash(id: string): string {
  return id.slice(0, 7)
}

/**
 * 「Copy commit info」的正文。
 *
 * 排版照着 `git show` 的头部走：贴进 issue 或聊天窗口时一眼能认出是什么。
 * hash 用完整的——缩写换个仓库、换个时间点都可能不再唯一。
 */
export function formatCommitInfo(commit: CommitNode): string {
  const date = new Date(commit.time * 1000).toLocaleString()
  return [
    `commit ${commit.id}`,
    `Author: ${commit.author}`,
    `Date:   ${date}`,
    '',
    commit.message,
  ].join('\n')
}

/**
 * 生成 graph 行的右键菜单。
 *
 * 返回的是**描述**而不是带回调的菜单项：这一层不碰 React、不碰 invoke，
 * 顺序、置灰条件和确认文案都能直接单测——这三样恰恰是最容易改坏的部分。
 */
export function commitMenu(commit: CommitNode, ctx: CommitMenuContext): CommitMenuEntry[] {
  const short = shortHash(commit.id)
  const all: CommitMenuEntry[] = [
    { action: 'new-branch', label: 'New Branch Here…' },
    { action: 'new-tag', label: 'New Tag Here…' },
    {
      action: 'checkout',
      label: `Checkout ${short}…`,
      // 已经站在这条提交上时没有可做的事，但保留位置
      disabled: ctx.isHead,
      // 游离 HEAD 是这里最容易让人懵的后果：界面看着切过去了，
      // 之后的提交却不属于任何分支，切走就找不回来了
      confirm: `Checkout ${short}?\n\nThis leaves HEAD detached — new commits won't belong to any branch.`,
    },
    {
      action: 'cherry-pick',
      label: `Cherry Pick ${short}…`,
      confirm: `Cherry-pick ${short} onto the current branch?\n\n${commit.message}`,
    },
    {
      action: 'revert',
      label: `Revert ${short}…`,
      confirm: `Revert ${short}?\n\nThis adds a new commit that undoes:\n${commit.message}`,
    },
    { action: 'save-patch', label: 'Save as Patch…' },
    { action: 'copy-hash', label: 'Copy Commit Hash' },
    { action: 'copy-info', label: 'Copy Commit Info' },
  ]

  // 逐项独立判断，不是「一关全关」——将来 SVN 后端可能只关掉其中一部分。
  const enabled: Record<string, boolean> = {
    'new-tag': ctx.caps.tags,
    'cherry-pick': ctx.caps.cherryPick,
    'save-patch': ctx.caps.patches,
  }
  return all.filter((e) => enabled[e.action] ?? true)
}

/**
 * 把菜单项翻译成一次后端调用。
 *
 * 返回 `null` 表示这一项还缺参数（要先弹输入框或存盘框）或者根本不走后端
 * （复制类）——调用方自己处理，不要在这一层瞎猜。
 */
export function planCommitCommand(
  action: CommitAction,
  commit: CommitNode,
): CommitCommandPlan | null {
  switch (action) {
    case 'checkout':
      // 一律用完整 hash：短 hash 在大仓库里可能撞上，git 会拒绝
      return { command: 'checkout', args: { rev: commit.id } }
    case 'cherry-pick':
      return { command: 'cherry_pick', args: { rev: commit.id } }
    case 'revert':
      return { command: 'revert', args: { rev: commit.id } }
    default:
      return null
  }
}
