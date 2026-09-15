import { describe, expect, it } from 'vitest'
import {
  isLocalOnly,
  parseGitSvnId,
  spliceHistory,
  type SvnRevNode,
} from '@/engine/svnGraph'
import type { CommitNode } from '@/engine/types'

const UUID = '8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01'
const ROOT = 'https://svn.company.com/game'

const svn = (rev: number): SvnRevNode => ({
  rev,
  author: 'someone',
  date: '2026-09-15T15:21:22.337313Z',
  message: `r${rev}`,
})

const git = (id: string, parents: string[], message = 'work'): CommitNode => ({
  id,
  parents,
  author: 'me',
  time: 1_700_000_000,
  message,
})

/** 一个真正的 baseline commit：message 带合法的 git-svn-id trailer。 */
const baseline = (id: string, parents: string[], rev: number): CommitNode =>
  git(id, parents, `svn: r${rev}\n\ngit-svn-id: ${ROOT}@${rev} ${UUID}`)

describe('parseGitSvnId', () => {
  it('三个字段缺一不可——解析失败即视为不是 baseline', () => {
    // spec §6.4.1：不猜、不部分接受。半解析出来的 trailer 会让一个普通 commit
    // 被当成基线从图上删掉。
    expect(parseGitSvnId(`svn: r12\n\ngit-svn-id: ${ROOT}@12 ${UUID}`)?.rev).toBe(12)
    expect(parseGitSvnId(`svn: r12\n\ngit-svn-id: ${ROOT} ${UUID}`)).toBeNull()
    expect(parseGitSvnId(`svn: r12\n\ngit-svn-id: @12 ${UUID}`)).toBeNull()
    expect(parseGitSvnId(`svn: r12\n\ngit-svn-id: ${ROOT}@abc ${UUID}`)).toBeNull()
    expect(parseGitSvnId(`svn: r12\n\ngit-svn-id: ${ROOT}@12`)).toBeNull()
  })

  it('取出 url 与 uuid', () => {
    const got = parseGitSvnId(`svn: r12\n\ngit-svn-id: ${ROOT}@12 ${UUID}`)!
    expect(got.url).toBe(ROOT)
    expect(got.uuid).toBe(UUID)
  })

  it('只认独立成行的 trailer，不匹配正文里顺口提到的', () => {
    // 用 includes 写会把这条误判成 baseline，于是它从图上消失。
    expect(parseGitSvnId(`聊到了 git-svn-id: ${ROOT}@1 ${UUID} 这个格式`)).toBeNull()
  })

  it('普通 commit 返回 null', () => {
    expect(parseGitSvnId('fix: 修一个空引用')).toBeNull()
    expect(parseGitSvnId('')).toBeNull()
  })
})

describe('isLocalOnly', () => {
  it('只认 Local-Only: true 这一条 trailer（spec §6.7）', () => {
    expect(isLocalOnly('local: mute spawn log\n\nLocal-Only: true')).toBe(true)
    expect(isLocalOnly('local: mute spawn log\n\nLocal-Only: false')).toBe(false)
    expect(isLocalOnly('plain commit')).toBe(false)
  })

  it('正文里提到不算', () => {
    expect(isLocalOnly('讨论了 Local-Only: true 要怎么写\n\n真正的结尾')).toBe(false)
  })

  it('容忍行尾空白与大小写', () => {
    // git 自己对 trailer 的键是大小写不敏感的，用户手打时大小写会飘。
    expect(isLocalOnly('x\n\nLocal-Only: true  ')).toBe(true)
    expect(isLocalOnly('x\n\nlocal-only: TRUE')).toBe(true)
  })
})

describe('spliceHistory', () => {
  it('SVN 序列连成一条链，parent 指向下一个更小的 revision', () => {
    const out = spliceHistory([svn(3), svn(2), svn(1)], [])
    expect(out.map((c) => c.id)).toEqual(['svn:3', 'svn:2', 'svn:1'])
    expect(out[0].parents).toEqual(['svn:2'])
    expect(out[1].parents).toEqual(['svn:1'])
    expect(out[2].parents).toEqual([])
  })

  it('svn 节点带上 vcs 与 svnRev', () => {
    const out = spliceHistory([svn(3)], [])
    expect(out[0].vcs).toBe('svn')
    expect(out[0].svnRev).toBe(3)
    expect(out[0].author).toBe('someone')
  })

  it('revision 不连号时照样接成链', () => {
    // 白名单路径的历史天然跳号：只动了别的目录的 revision 不会出现在这里。
    const out = spliceHistory([svn(9), svn(4), svn(1)], [])
    expect(out[0].parents).toEqual(['svn:4'])
    expect(out[1].parents).toEqual(['svn:1'])
  })

  it('baseline commit 被替换掉，指向它的 git commit 改指 svn 节点', () => {
    // spec §10.3 ③：baseline commit 自身被替换掉，它在图上的位置由 SVN 序列决定。
    const out = spliceHistory([svn(3), svn(2)], [git('aaa', ['bbb']), baseline('bbb', ['ccc'], 3)])
    expect(out.find((c) => c.id === 'bbb')).toBeUndefined()
    expect(out.find((c) => c.id === 'aaa')!.parents).toEqual(['svn:3'])
  })

  it('孤儿 baseline —— 锚点没拉到时不留悬空父', () => {
    // 悬空父不会画出线（computePaths 里 `if (!parentNode) continue`），
    // 但 computeOwners 仍然拿它算归属，会把车道分配算歪。
    //
    // 锚点补拉走网络，断网时拿不到，所以这**不是**理论情况。
    const out = spliceHistory([svn(2)], [git('aaa', ['bbb']), baseline('bbb', [], 99)])
    const aaa = out.find((c) => c.id === 'aaa')!
    expect(aaa.parents).toEqual([])
    expect(out.every((c) => c.parents.every((p) => out.some((n) => n.id === p)))).toBe(true)
  })

  it('普通 git commit 之间的 parent 关系原样保留', () => {
    const out = spliceHistory([], [git('aaa', ['bbb']), git('bbb', [])])
    expect(out.find((c) => c.id === 'aaa')!.parents).toEqual(['bbb'])
  })

  it('合并提交的多个父各自替换', () => {
    const out = spliceHistory(
      [svn(5)],
      [git('merge', ['aaa', 'bbb']), git('aaa', []), baseline('bbb', [], 5)],
    )
    expect(out.find((c) => c.id === 'merge')!.parents).toEqual(['aaa', 'svn:5'])
  })

  it('localOnly 按 trailer 标出来，普通 commit 不标', () => {
    const out = spliceHistory([], [git('aaa', [], 'local: x\n\nLocal-Only: true'), git('bbb', [])])
    expect(out.find((c) => c.id === 'aaa')!.localOnly).toBe(true)
    expect(out.find((c) => c.id === 'bbb')!.localOnly).toBe(false)
  })

  it('git 节点标成 vcs=git', () => {
    const out = spliceHistory([], [git('aaa', [])])
    expect(out[0].vcs).toBe('git')
  })

  it('纯 git 仓库（svnRevs 为空）时拓扑与输入等价', () => {
    // spec §10.4：现有 Git 模式零影响。
    const input = [git('aaa', ['bbb']), git('bbb', ['ccc']), git('ccc', [])]
    const out = spliceHistory([], input)
    expect(out.map((c) => ({ id: c.id, parents: c.parents }))).toEqual(
      input.map((c) => ({ id: c.id, parents: c.parents })),
    )
  })

  it('svn 节点排在 git 节点之后，整体仍是「新的在前」', () => {
    // 工作分支的提交比基线新，图上要在上面。
    const out = spliceHistory([svn(2), svn(1)], [git('aaa', ['bbb']), baseline('bbb', [], 2)])
    expect(out.map((c) => c.id)).toEqual(['aaa', 'svn:2', 'svn:1'])
  })

  it('两个 baseline 指向同一个 revision 时都被替换', () => {
    // 重建基线会在 svn/base 上落新 commit，旧的仍在历史里。
    const out = spliceHistory(
      [svn(4)],
      [git('aaa', ['b1']), baseline('b1', ['b0'], 4), baseline('b0', [], 4)],
    )
    expect(out.find((c) => c.id === 'b1')).toBeUndefined()
    expect(out.find((c) => c.id === 'b0')).toBeUndefined()
    expect(out.find((c) => c.id === 'aaa')!.parents).toEqual(['svn:4'])
  })

  it('空输入返回空数组', () => {
    expect(spliceHistory([], [])).toEqual([])
  })
})
