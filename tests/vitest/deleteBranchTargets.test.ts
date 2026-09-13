import { describe, expect, it } from 'vitest'
import { ALL_CAPABILITIES, NO_GIT_ONLY_CAPABILITIES } from '@/engine/capabilities'
import { deleteBranchTargets, type DeleteBranchInfo } from '@/engine/deleteBranchTargets'

/** 有上游、也有 worktree 的分支——两个勾选框都会出现的形态。 */
const info: DeleteBranchInfo = {
  upstream: 'origin/feat/x',
  remote: 'origin',
  remoteBranch: 'feat/x',
  remoteProtected: false,
  worktree: { path: 'C:/wt/feat-x' },
}

const bothChecked = { alsoRemote: true, alsoWorktree: true }

describe('deleteBranchTargets', () => {
  it('普通 git 仓库：两项都勾上时两个目标都发出去', () => {
    expect(deleteBranchTargets(ALL_CAPABILITIES, info, bothChecked)).toEqual({
      remote: { remote: 'origin', branch: 'feat/x' },
      worktreePath: 'C:/wt/feat-x',
    })
  })

  it('后端不支持 remotes 时绝不构造远程删除目标', () => {
    // 这是独立 review 抓到的那一条：上一轮只把 worktree 那半接上了
    // `capabilities.worktrees`，远程这半仍然只看 `info.upstream`。混合仓库的
    // git 侧完全可以留着 remote 和上游（当前的打开校验并不禁止），于是
    // **正常的删除确认流程**就能调到一个声明为不支持的能力——这不是 PR 里
    // 已披露的「绕过 UI 直接 invoke」，是 UI 自己走进去的。
    expect(deleteBranchTargets(NO_GIT_ONLY_CAPABILITIES, info, bothChecked)).toEqual({
      remote: null,
      worktreePath: null,
    })
  })

  it('两种能力互不牵连', () => {
    const noWorktree = { ...ALL_CAPABILITIES, worktrees: false }
    expect(deleteBranchTargets(noWorktree, info, bothChecked)).toEqual({
      remote: { remote: 'origin', branch: 'feat/x' },
      worktreePath: null,
    })
    const noRemote = { ...ALL_CAPABILITIES, remotes: false }
    expect(deleteBranchTargets(noRemote, info, bothChecked)).toEqual({
      remote: null,
      worktreePath: 'C:/wt/feat-x',
    })
  })

  it('受保护的远程分支照旧不发，即使能力开着', () => {
    const protectedInfo = { ...info, remoteProtected: true }
    expect(deleteBranchTargets(ALL_CAPABILITIES, protectedInfo, bothChecked).remote).toBeNull()
  })

  it('没勾就不发', () => {
    expect(
      deleteBranchTargets(ALL_CAPABILITIES, info, { alsoRemote: false, alsoWorktree: false }),
    ).toEqual({ remote: null, worktreePath: null })
  })

  it('信息缺失时不硬凑目标', () => {
    // remote / remoteBranch 任一为 null 都不能发——拼出半个坐标比不发更糟
    const half: DeleteBranchInfo = { ...info, remoteBranch: null }
    expect(deleteBranchTargets(ALL_CAPABILITIES, half, bothChecked).remote).toBeNull()
    const noWt: DeleteBranchInfo = { ...info, worktree: null }
    expect(deleteBranchTargets(ALL_CAPABILITIES, noWt, bothChecked).worktreePath).toBeNull()
  })
})
