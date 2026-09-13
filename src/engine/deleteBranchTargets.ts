import type { Capabilities } from './capabilities'

/**
 * 删除分支确认框里、用来决定「这次都要动什么」的那部分信息。
 *
 * 只列真正参与决策的字段，`DeleteInfo` 结构上满足它。放在这里是为了让这段
 * 判断脱离组件、能直接单测（AGENTS.md §7.2）。
 */
export interface DeleteBranchInfo {
  /** 对应的远程分支简称；为 null 时不显示远程勾选框。 */
  upstream: string | null
  /** 上游所在的远程名。 */
  remote: string | null
  /** 该分支在远程上的名字。 */
  remoteBranch: string | null
  /** 那条远程分支是该远程的默认分支——不提供删除。 */
  remoteProtected: boolean
  /** 检出了这个分支的 worktree；为 null 时不显示那个勾选框。 */
  worktree: { path: string } | null
}

/** 用户在两个勾选框上的选择。 */
export interface DeleteBranchChoice {
  alsoRemote: boolean
  alsoWorktree: boolean
}

/** 这一次删除真正会发出去的目标。 */
export interface DeleteBranchTargets {
  remote: { remote: string; branch: string } | null
  worktreePath: string | null
}

/**
 * 算出这次删除真正要发出去的目标。
 *
 * # 为什么是一个独立的纯函数
 *
 * 因为「渲染没渲染那个勾选框」和「发不发那个动作」是两件事，而把它们分散在
 * 组件里写过一次就已经出过事：2026-09-13 的独立 review（A5）抓到 worktree
 * 那半没判能力，我修了；2026-09-14 的这一轮又抓到**远程那半同样没判**——
 * 同一个 `confirm()` 里，我只接上了一半。
 *
 * 分散写的问题不是粗心，是没有一个地方能一眼看全。这里把两个目标的构造收到
 * 一处：往后再加一个「同时删 X」，改的是这个函数，而它是有测试的。
 *
 * ⚠️ 这仍然是**表现层**防线。后端命令没有经过 `trait Vcs`，绕过 UI 直接
 * `invoke` 照样打得进去（见 `capabilities.ts` 的模块注释）。这里守的是
 * 「正常流程不会走进声明为不支持的能力」。
 */
export function deleteBranchTargets(
  caps: Capabilities,
  info: DeleteBranchInfo,
  choice: DeleteBranchChoice,
): DeleteBranchTargets {
  const remote =
    caps.remotes && choice.alsoRemote && !info.remoteProtected && info.remote && info.remoteBranch
      ? { remote: info.remote, branch: info.remoteBranch }
      : null
  const worktreePath =
    caps.worktrees && choice.alsoWorktree && info.worktree ? info.worktree.path : null
  return { remote, worktreePath }
}
