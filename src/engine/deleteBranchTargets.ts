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

/** 用户在两个勾选框上的选择，加上探测对远程那条分支的结论。 */
export interface DeleteBranchChoice {
  alsoRemote: boolean
  alsoWorktree: boolean
  /** 探测明确答了「远程上已经没有这条分支」。没探测过、问不到都是 false。 */
  remoteGone?: boolean
}

/** 这一次删除真正会发出去的目标。 */
export interface DeleteBranchTargets {
  remote: { remote: string; branch: string } | null
  worktreePath: string | null
  /**
   * 远程上已经没有的那条远程分支：删除做完后清掉它在本地的过期跟踪 ref。
   *
   * 不在探测时就清：那时用户还没确认任何删除，清掉之后弹框的重查还会退到别的远程上的
   * 同名分支，勾选框就指向一条用户没选过的分支。
   */
  staleRemote: { remote: string; branch: string } | null
}

/** 远程分支坐标的字符串形式，当 effect 依赖与比较用。弹框里所有比较都走它，免得两处拼法不一致。 */
export function remoteKey(target: { remote: string; branch: string } | null): string | null {
  return target ? `${target.remote}\0${target.branch}` : null
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
  const coord = caps.remotes && info.remote && info.remoteBranch
    ? { remote: info.remote, branch: info.remoteBranch }
    : null
  // 远程上已经没有就不再删：push 只会白握一次手
  const remote = coord && choice.alsoRemote && !choice.remoteGone && !info.remoteProtected ? coord : null
  const staleRemote = coord && choice.remoteGone ? coord : null
  const worktreePath =
    caps.worktrees && choice.alsoWorktree && info.worktree ? info.worktree.path : null
  return { remote, worktreePath, staleRemote }
}

/** 做完就回不了头、重试时不能再做一遍的步骤。本地分支是最后一步，它做完序列就结束了。 */
export type BranchDeletionStep = 'worktree' | 'remote'

/** 删除序列里每一步真正的动作，由调用方接到 `invoke` 上。 */
export interface BranchDeletionSteps {
  /** 只跑远程护栏，不删任何东西。被拒时抛错。 */
  checkRemote: (remote: NonNullable<DeleteBranchTargets['remote']>) => Promise<void>
  deleteWorktree: (worktreePath: string) => Promise<void>
  deleteRemote: (remote: NonNullable<DeleteBranchTargets['remote']>) => Promise<void>
  deleteLocal: () => Promise<void>
  /** 清掉一条远程已经没有的分支在本地的过期跟踪 ref。只读本地配置，不走网络。失败只记日志。 */
  forgetGoneRemote: (remote: { remote: string; branch: string }) => Promise<void>
}

/**
 * 按顺序跑完一次删除，任何一步失败都抛出去，后面的步骤不再执行。
 *
 * **顺序是 worktree → 远程 → 本地分支**，按「最容易失败的放最前」排：
 *
 * - worktree 那一步最常被拒——只要里面有改动或未跟踪文件，`git worktree
 *   remove` 就不干（这也意味着它不会偷偷毁掉未提交的东西）。它排第一，
 *   失败时远程和本地都还没动过，原地重试就行。
 * - 远程那一步会因为外部原因失败（断网、没权限、分支被保护），排在删本地
 *   引用之前；反过来的话本地引用已经没了，还得先去 reflog 里捞回来才能重试。
 * - 本地分支最后。它也**必须**在 worktree 之后：分支被 worktree 检出时
 *   `git branch -D` 会直接拒绝。
 *
 * **worktree 和远程都要删时先预检远程。** worktree 删了就回不来，而远程护栏的判断
 * 确定得很：`git remote add` 接上的远程没有本地的 `origin/HEAD`，确认框的勾选框不会
 * 灰掉，不预检的话要到第二步才被拒，用户得到一次部分删除。没有 worktree 时不预检：
 * 删远程本身就是第一步，护栏会先拦，再问一次只是白跑。预检只防护栏拒绝，push 自身的
 * 失败（断网、分支已经不在）防不了，那时确认框留在原地，靠 `onStepDone` 记下的进度只重试
 * 没做完的步骤。
 */
export async function runBranchDeletion(
  { remote, worktreePath, staleRemote }: DeleteBranchTargets,
  onStepDone: (step: BranchDeletionStep) => void,
  steps: BranchDeletionSteps,
): Promise<void> {
  if (worktreePath && remote) await steps.checkRemote(remote)
  if (worktreePath) {
    await steps.deleteWorktree(worktreePath)
    onStepDone('worktree')
  }
  if (remote) {
    await steps.deleteRemote(remote)
    onStepDone('remote')
  }
  await steps.deleteLocal()
  // 删除已经全部完成，清理只是顺手：失败也不能抛出去，否则确认框会把它显示成删除失败
  if (staleRemote) {
    await steps.forgetGoneRemote(staleRemote).catch((err) => {
      console.warn('清理过期的远程跟踪分支失败：', err)
    })
  }
}

/**
 * 去掉已经做完的步骤对应的目标：重试时不再发，界面上也不再提供。
 *
 * 比「失败后重查后端」可靠：重查只知道现状，不知道用户选过什么，见 `DeleteBranchDialog`。
 */
export function withoutCompletedSteps<T extends DeleteBranchInfo>(
  info: T,
  completed: readonly BranchDeletionStep[],
): T {
  const withoutWorktree = completed.includes('worktree') ? { ...info, worktree: null } : info
  return completed.includes('remote')
    ? {
        ...withoutWorktree,
        upstream: null,
        remote: null,
        remoteBranch: null,
        remoteProtected: false,
      }
    : withoutWorktree
}
