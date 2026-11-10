import { describe, expect, it, vi } from 'vitest'
import { ALL_CAPABILITIES, NO_GIT_ONLY_CAPABILITIES } from '@/engine/capabilities'
import {
  deleteBranchTargets,
  remoteKey,
  runBranchDeletion,
  withoutCompletedSteps,
  type DeleteBranchInfo,
} from '@/engine/deleteBranchTargets'

const remoteTarget = { remote: 'origin', branch: 'feat/x' }

/** 记下实际发生的步骤；`fail` 里的步骤抛错，模拟那一步被拒。 */
function recordSteps(fail?: string) {
  const calls: string[] = []
  const step = (label: string) => vi.fn(async () => {
    calls.push(label)
    if (label === fail) throw new Error(`${label} 被拒`)
  })
  return {
    calls,
    steps: {
      checkRemote: step('checkRemote'),
      deleteWorktree: step('deleteWorktree'),
      deleteRemote: step('deleteRemote'),
      deleteLocal: step('deleteLocal'),
      forgetGoneRemote: step('forgetGoneRemote'),
    },
  }
}

describe('runBranchDeletion 清理远程已经没有的分支', () => {
  it('删完本地分支之后才清：用户确认的删除做完之前不碰跟踪 ref', async () => {
    const { calls, steps } = recordSteps()
    await runBranchDeletion(
      { remote: null, worktreePath: 'C:/wt/x', staleRemote: remoteTarget },
      vi.fn(),
      steps,
    )
    expect(calls).toEqual(['deleteWorktree', 'deleteLocal', 'forgetGoneRemote'])
    expect(steps.forgetGoneRemote).toHaveBeenCalledWith(remoteTarget)
  })

  it('清理失败不算删除失败：删除已经做完了', async () => {
    const { calls, steps } = recordSteps('forgetGoneRemote')
    await runBranchDeletion({ remote: null, worktreePath: null, staleRemote: remoteTarget }, vi.fn(), steps)
    expect(calls).toEqual(['deleteLocal', 'forgetGoneRemote'])
  })

  it('本地分支没删成：不清理', async () => {
    const { calls, steps } = recordSteps('deleteLocal')
    await expect(
      runBranchDeletion({ remote: null, worktreePath: null, staleRemote: remoteTarget }, vi.fn(), steps),
    ).rejects.toThrow()
    expect(calls).toEqual(['deleteLocal'])
  })
})

describe('deleteBranchTargets 远程已经没有时', () => {
  const info: DeleteBranchInfo = {
    upstream: 'origin/feat/x',
    remote: 'origin',
    remoteBranch: 'feat/x',
    remoteProtected: false,
    worktree: null,
  }
  const gone = { alsoRemote: true, alsoWorktree: false, remoteGone: true }

  it('不再删远程（只会白握一次手），删完后清过期的跟踪 ref', () => {
    expect(deleteBranchTargets(ALL_CAPABILITIES, info, gone)).toEqual({
      remote: null,
      worktreePath: null,
      staleRemote: remoteTarget,
    })
  })

  it('没探测过或问不到：照常删远程，不清', () => {
    const t = deleteBranchTargets(ALL_CAPABILITIES, info, { ...gone, remoteGone: false })
    expect(t.remote).toEqual(remoteTarget)
    expect(t.staleRemote).toBeNull()
  })

  it('不管理远程的模式不清', () => {
    expect(deleteBranchTargets(NO_GIT_ONLY_CAPABILITIES, info, gone).staleRemote).toBeNull()
  })

  it('没有远程分支坐标时不清', () => {
    expect(deleteBranchTargets(ALL_CAPABILITIES, { ...info, remote: null }, gone).staleRemote).toBeNull()
  })

  it('remoteKey：同一坐标同一个键，没有坐标就是 null', () => {
    expect(remoteKey(remoteTarget)).toBe(remoteKey({ ...remoteTarget }))
    expect(remoteKey(remoteTarget)).not.toBe(remoteKey({ remote: 'upstream', branch: 'feat/x' }))
    expect(remoteKey(null)).toBeNull()
  })
})

describe('runBranchDeletion', () => {
  it('worktree 和远程都要删时，先预检远程，再按 worktree → 远程 → 本地走', async () => {
    const { calls, steps } = recordSteps()
    await runBranchDeletion({ remote: remoteTarget, worktreePath: 'C:/wt/x' }, vi.fn(), steps)
    expect(calls).toEqual(['checkRemote', 'deleteWorktree', 'deleteRemote', 'deleteLocal'])
    expect(steps.checkRemote).toHaveBeenCalledWith(remoteTarget)
    expect(steps.deleteWorktree).toHaveBeenCalledWith('C:/wt/x')
    expect(steps.deleteRemote).toHaveBeenCalledWith(remoteTarget)
  })

  it('预检被拒时什么都不删：worktree 删了就回不来', async () => {
    // 这就是要修的缺陷：`git remote add` 接上的远程没有本地 origin/HEAD，勾选框不会灰掉，
    // 以前 worktree 删完才被护栏拒绝，用户得到一次部分删除
    const { calls, steps } = recordSteps('checkRemote')
    await expect(
      runBranchDeletion({ remote: remoteTarget, worktreePath: 'C:/wt/x' }, vi.fn(), steps),
    ).rejects.toThrow('checkRemote 被拒')
    expect(calls).toEqual(['checkRemote'])
  })

  it('没有 worktree 时不预检：删远程本身就是第一步，护栏会先拦，预检只会白跑', async () => {
    const { calls, steps } = recordSteps()
    await runBranchDeletion({ remote: remoteTarget, worktreePath: null }, vi.fn(), steps)
    expect(calls).toEqual(['deleteRemote', 'deleteLocal'])
  })

  it('不删远程时不预检，不走网络', async () => {
    const { calls, steps } = recordSteps()
    await runBranchDeletion({ remote: null, worktreePath: 'C:/wt/x' }, vi.fn(), steps)
    expect(calls).toEqual(['deleteWorktree', 'deleteLocal'])
  })

  it('两个都不删时只删本地分支', async () => {
    const { calls, steps } = recordSteps()
    await runBranchDeletion({ remote: null, worktreePath: null }, vi.fn(), steps)
    expect(calls).toEqual(['deleteLocal'])
  })

  it('某一步失败后不再往下走', async () => {
    const worktree = recordSteps('deleteWorktree')
    await expect(
      runBranchDeletion({ remote: remoteTarget, worktreePath: 'C:/wt/x' }, vi.fn(), worktree.steps),
    ).rejects.toThrow()
    expect(worktree.calls).toEqual(['checkRemote', 'deleteWorktree'])

    const remote = recordSteps('deleteRemote')
    await expect(
      runBranchDeletion({ remote: remoteTarget, worktreePath: null }, vi.fn(), remote.steps),
    ).rejects.toThrow()
    expect(remote.calls).toEqual(['deleteRemote'])
  })

  it('每做完一步上报一次，失败的那步和没走到的不算', async () => {
    const all = vi.fn()
    await runBranchDeletion(
      { remote: remoteTarget, worktreePath: 'C:/wt/x' },
      all,
      recordSteps().steps,
    )
    expect(all.mock.calls.map(([step]) => step)).toEqual(['worktree', 'remote'])

    const partial = vi.fn()
    await expect(
      runBranchDeletion(
        { remote: remoteTarget, worktreePath: 'C:/wt/x' },
        partial,
        recordSteps('deleteRemote').steps,
      ),
    ).rejects.toThrow()
    expect(partial.mock.calls.map(([step]) => step)).toEqual(['worktree'])
  })
})

describe('withoutCompletedSteps', () => {
  const full: DeleteBranchInfo = {
    upstream: 'origin/feat/x',
    remote: 'origin',
    remoteBranch: 'feat/x',
    remoteProtected: false,
    worktree: { path: 'C:/wt/feat-x' },
  }
  const choice = { alsoRemote: true, alsoWorktree: true }

  it('什么都没做完时原样返回', () => {
    expect(withoutCompletedSteps(full, [])).toEqual(full)
  })

  it('worktree 做完了：重试只剩远程', () => {
    const left = withoutCompletedSteps(full, ['worktree'])
    expect(deleteBranchTargets(ALL_CAPABILITIES, left, choice)).toEqual({
      remote: { remote: 'origin', branch: 'feat/x' },
      worktreePath: null,
      staleRemote: null,
    })
  })

  it('远程做完了：重试只剩 worktree，勾选框的依据也一起清掉', () => {
    const left = withoutCompletedSteps(full, ['remote'])
    expect(deleteBranchTargets(ALL_CAPABILITIES, left, choice)).toEqual({
      remote: null,
      worktreePath: 'C:/wt/feat-x',
      staleRemote: null,
    })
    expect(left.upstream).toBeNull()
  })

  it('都做完了：重试只删本地分支', () => {
    const left = withoutCompletedSteps(full, ['worktree', 'remote'])
    expect(deleteBranchTargets(ALL_CAPABILITIES, left, choice)).toEqual({
      remote: null,
      worktreePath: null,
      staleRemote: null,
    })
  })
})

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
      staleRemote: null,
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
      staleRemote: null,
    })
  })

  it('两种能力互不牵连', () => {
    const noWorktree = { ...ALL_CAPABILITIES, worktrees: false }
    expect(deleteBranchTargets(noWorktree, info, bothChecked)).toEqual({
      remote: { remote: 'origin', branch: 'feat/x' },
      worktreePath: null,
      staleRemote: null,
    })
    const noRemote = { ...ALL_CAPABILITIES, remotes: false }
    expect(deleteBranchTargets(noRemote, info, bothChecked)).toEqual({
      remote: null,
      worktreePath: 'C:/wt/feat-x',
      staleRemote: null,
    })
  })

  it('受保护的远程分支照旧不发，即使能力开着', () => {
    const protectedInfo = { ...info, remoteProtected: true }
    expect(deleteBranchTargets(ALL_CAPABILITIES, protectedInfo, bothChecked).remote).toBeNull()
  })

  it('没勾就不发', () => {
    expect(
      deleteBranchTargets(ALL_CAPABILITIES, info, { alsoRemote: false, alsoWorktree: false }),
    ).toEqual({ remote: null, worktreePath: null, staleRemote: null })
  })

  it('信息缺失时不硬凑目标', () => {
    // remote / remoteBranch 任一为 null 都不能发——拼出半个坐标比不发更糟
    const half: DeleteBranchInfo = { ...info, remoteBranch: null }
    expect(deleteBranchTargets(ALL_CAPABILITIES, half, bothChecked).remote).toBeNull()
    const noWt: DeleteBranchInfo = { ...info, worktree: null }
    expect(deleteBranchTargets(ALL_CAPABILITIES, noWt, bothChecked).worktreePath).toBeNull()
  })
})
