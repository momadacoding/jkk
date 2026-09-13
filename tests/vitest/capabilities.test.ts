import { describe, expect, it } from 'vitest'
import { ALL_CAPABILITIES, NO_GIT_ONLY_CAPABILITIES, visibleSections } from '@/engine/capabilities'
import { visibleToolbarActions } from '@/engine/toolbarActions'
import { commitMenu } from '@/engine/commitActions'
import type { CommitNode } from '@/engine/types'

const commit: CommitNode = {
  id: 'd086703aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  parents: [],
  author: 'someone',
  time: 1_700_000_000,
  message: 'do a thing',
}

describe('visibleSections', () => {
  it('普通 git 仓库渲染全部侧边栏分区，顺序与 RepoTab 的渲染顺序一致', () => {
    // 顺序也断言：这个数组就是拿来驱动渲染的，顺序错了侧边栏就换了个样子
    expect(visibleSections(ALL_CAPABILITIES)).toEqual([
      'branches',
      'tags',
      'worktrees',
      'stashes',
    ])
  })

  it('SVN 模式只留下分支与 stash', () => {
    expect(visibleSections(NO_GIT_ONLY_CAPABILITIES)).toEqual(['branches', 'stashes'])
  })

  it('分支与 stash 没有对应的 capability 开关——它们在两种模式下都可用', () => {
    // §16 Phase 1 验收：「本地 git 的暂存/分支/stash 可用」。
    // 如果哪天给它们加了开关，这条会提醒去改验收标准而不是顺手关掉。
    expect(Object.keys(ALL_CAPABILITIES).sort()).toEqual([
      'cherryPick',
      'patches',
      'pullRequests',
      'remotes',
      'tags',
      'worktrees',
    ])
  })

  it('两个常量的键完全一致——漏一个键就是漏一个没被关掉的入口', () => {
    expect(Object.keys(NO_GIT_ONLY_CAPABILITIES).sort()).toEqual(
      Object.keys(ALL_CAPABILITIES).sort(),
    )
    expect(Object.values(ALL_CAPABILITIES).every((v) => v === true)).toBe(true)
    expect(Object.values(NO_GIT_ONLY_CAPABILITIES).every((v) => v === false)).toBe(true)
  })
})

describe('visibleToolbarActions', () => {
  it('普通 git 仓库四个按钮都在', () => {
    expect(visibleToolbarActions(ALL_CAPABILITIES)).toEqual(['fetch', 'pull', 'push', 'stash'])
  })

  it('SVN 模式只留 stash——fetch/pull/push 全都直连 git remote', () => {
    // 这三个命令没有进 trait Vcs 的接缝，仍然直接 run_git。SVN 工作副本上
    // 它们要么报「没有远程」，要么对着一个别人留下的 remote 真的动起来。
    expect(visibleToolbarActions(NO_GIT_ONLY_CAPABILITIES)).toEqual(['stash'])
  })
})

describe('commitMenu 按能力裁剪', () => {
  const actions = (caps = ALL_CAPABILITIES) =>
    commitMenu(commit, { isHead: false, caps }).map((e) => e.action)

  it('普通 git 仓库给齐 8 项', () => {
    expect(actions()).toHaveLength(8)
  })

  it('SVN 模式去掉 new-tag / cherry-pick / save-patch，其余保持原顺序', () => {
    // 这三项分别依赖 git 的 tag、DAG 语义和 format-patch，SVN 侧没有对应物；
    // 而它们都**没有**进 trait Vcs 的接缝，点下去就是直接对工作目录做 git 操作。
    expect(actions(NO_GIT_ONLY_CAPABILITIES)).toEqual([
      'new-branch',
      'checkout',
      'revert',
      'copy-hash',
      'copy-info',
    ])
  })

  it('裁剪是逐项独立的，不是「一关全关」', () => {
    const onlyTags = { ...NO_GIT_ONLY_CAPABILITIES, tags: true }
    expect(actions(onlyTags)).toContain('new-tag')
    expect(actions(onlyTags)).not.toContain('cherry-pick')
  })
})
