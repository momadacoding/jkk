import { describe, expect, it } from 'vitest'
import {
  ALL_CAPABILITIES,
  NO_GIT_ONLY_CAPABILITIES,
  isBaselineBranch,
  isProtectedBaseline,
  visibleSections,
} from '@/engine/capabilities'
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
    //
    // 这份键列表必须与 Rust 侧 `vcs::Capabilities` 序列化出来的键**逐字一致**，
    // 那一侧由 `tests/svn_backend.rs` 的
    // `capabilities_serializes_to_the_exact_field_names_the_frontend_reads` 钉住。
    expect(Object.keys(ALL_CAPABILITIES).sort()).toEqual([
      'cherryPick',
      'patches',
      'pullRequests',
      'remotes',
      'tags',
      'vcs',
      'worktrees',
    ])
  })

  it('识别基线分支，容忍 refs/heads/ 前缀', () => {
    expect(isBaselineBranch('svn/base')).toBe(true)
    expect(isBaselineBranch('refs/heads/svn/base')).toBe(true)
  })

  it('不把名字相近的分支当成基线', () => {
    // 这些都是合法分支名。误判会让用户在自己的备份分支上发现「不能提交」，
    // 而且没有任何解释。
    expect(isBaselineBranch('svn/base-backup')).toBe(false)
    expect(isBaselineBranch('feature/svn/base')).toBe(false)
    expect(isBaselineBranch('svn/basement')).toBe(false)
    expect(isBaselineBranch('master')).toBe(false)
    expect(isBaselineBranch('')).toBe(false)
  })

  it('只在 SVN 混合仓库里把 svn/base 当基线护着', () => {
    // 普通 git 仓库里叫 `svn/base` 的分支只是用户自己的分支：后端 `GitBackend` 不拦，
    // 界面却把改名、删除、提交全置灰且不解释，等于凭空锁死了它
    expect(isProtectedBaseline('svn/base', NO_GIT_ONLY_CAPABILITIES)).toBe(true)
    expect(isProtectedBaseline('refs/heads/svn/base', NO_GIT_ONLY_CAPABILITIES)).toBe(true)
    expect(isProtectedBaseline('svn/base', ALL_CAPABILITIES)).toBe(false)
    expect(isProtectedBaseline('svn/base-backup', NO_GIT_ONLY_CAPABILITIES)).toBe(false)
  })

  it('vcs 说清楚背后是哪种版本控制，不靠别的字段反推', () => {
    // 反推（「所有 git 专属能力都是 false ⇒ 是 SVN」）在今天成立纯属巧合：
    // 将来某个普通 git 仓库恰好关掉全部可选能力时就会被误判，于是每次刷新
    // 都对它发几个必然失败的 SVN invoke。
    expect(ALL_CAPABILITIES.vcs).toBe('git')
    expect(NO_GIT_ONLY_CAPABILITIES.vcs).toBe('svn')
  })

  it('两个常量的键完全一致——漏一个键就是漏一个没被关掉的入口', () => {
    expect(Object.keys(NO_GIT_ONLY_CAPABILITIES).sort()).toEqual(
      Object.keys(ALL_CAPABILITIES).sort(),
    )
    // `vcs` 是字符串不是开关，单独排除掉再断言「其余全开 / 全关」。
    // 直接 `Object.values(...).every(v => v === true)` 在加了非布尔字段之后
    // 就永远为 false，那会把这条测试变成一句空话。
    const switches = (c: typeof ALL_CAPABILITIES) =>
      Object.entries(c).filter(([k]) => k !== 'vcs').map(([, v]) => v)
    expect(switches(ALL_CAPABILITIES).every((v) => v === true)).toBe(true)
    expect(switches(NO_GIT_ONLY_CAPABILITIES).every((v) => v === false)).toBe(true)
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
