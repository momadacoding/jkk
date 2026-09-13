import type { Capabilities } from './capabilities'

export type ToolbarAction = 'fetch' | 'pull' | 'push' | 'stash'

/** 工具栏按钮的完整顺序。顺序即肌肉记忆，不要改。 */
const ALL_ACTIONS: ToolbarAction[] = ['fetch', 'pull', 'push', 'stash']

/**
 * 按后端能力裁剪工具栏按钮，保持原顺序。
 *
 * `fetch` / `pull` / `push` 都直连 git remote，且**没有**进 `trait Vcs` 的接缝。
 * SVN 工作副本上它们要么报「没有远程」，要么对着一个别人留下的 remote 真的动起来。
 * `stash` 是本地 git 操作，两种模式下都留着（§16 Phase 1 验收）。
 */
export function visibleToolbarActions(caps: Capabilities): ToolbarAction[] {
  return ALL_ACTIONS.filter((a) => caps.remotes || a === 'stash')
}

export interface ToolbarPlan {
  /** 按钮上显示的文字。按住 Ctrl 时必须变化，否则变体功能无从发现。 */
  label: string
  /** 要 invoke 的 Tauri 命令名。 */
  command: string
  /** 除 path 之外的参数。 */
  args: Record<string, unknown>
  /** 非空时先弹确认框，文案即此字符串。 */
  confirm: string | null
}

/**
 * 把「哪个按钮 + 有没有按 Ctrl」翻译成一次具体的后端调用。
 *
 * 抽成纯函数是为了可测：本仓库没有 jsdom，组件跑不起来，
 * 但这段分支逻辑恰恰是最容易写错的部分。
 */
export function planToolbarAction(
  action: ToolbarAction,
  ctrl: boolean,
  branch: string,
): ToolbarPlan {
  switch (action) {
    case 'fetch':
      return {
        label: ctrl ? 'Fetch all' : 'Fetch',
        command: 'fetch',
        args: { all: ctrl },
        confirm: null,
      }
    case 'pull':
      return {
        label: ctrl ? 'Pull (rebase)' : 'Pull',
        command: 'pull',
        args: { rebase: ctrl },
        confirm: null,
      }
    case 'push':
      return {
        label: ctrl ? 'Force push' : 'Push',
        command: 'push',
        args: { force: ctrl },
        // 工具栏上唯一可能覆盖远程历史的动作，值得拦一道
        confirm: ctrl
          ? `Force-push ${branch}? This can overwrite commits on the remote.`
          : null,
      }
    case 'stash':
      return {
        label: ctrl ? 'Stash all' : 'Stash',
        command: 'stash_push',
        args: { message: null, includeUntracked: ctrl },
        confirm: null,
      }
  }
}
