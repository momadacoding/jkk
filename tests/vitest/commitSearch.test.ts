import { describe, expect, it } from 'vitest'
import {
  buildSearchIndex,
  findMatches,
  inputSearchKey,
  matchPosition,
  parseSearch,
  stepMatch,
  windowSearchKey,
  type SearchKeyEvent,
} from '@/engine/commitSearch'
import type { CommitNode, RefPointer } from '@/engine/types'

function commit(id: string, message: string, extra: Partial<CommitNode> = {}): CommitNode {
  return { id, parents: [], author: 'Alice', time: 0, message, ...extra }
}

function ref(name: string, hash: string, kind: RefPointer['kind'] = 'branch'): RefPointer {
  return { name, hash, kind, ahead: null, behind: null, goneUpstream: null }
}

const HASH_A = 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678'
const HASH_B = 'ffee00112233445566778899aabbccddeeff0011'

describe('parseSearch', () => {
  it('空串、只有空白：没有词', () => {
    expect(parseSearch('')).toEqual([])
    expect(parseSearch('   \t ')).toEqual([])
  })

  it('按空白拆词、转小写', () => {
    expect(parseSearch('  Fix  Login ')).toEqual(['fix', 'login'])
  })
})

describe('findMatches', () => {
  const commits = [
    commit(HASH_A, 'Fix login redirect', { author: 'Bob Smith' }),
    commit(HASH_B, 'Add search bar'),
  ]
  const refs = [ref('feature/login', HASH_B), ref('v1.0', HASH_A, 'tag')]
  const index = buildSearchIndex(commits, refs)
  const run = (q: string) => findMatches(index, parseSearch(q))

  it('没有词时没有命中（不是全部命中）', () => {
    expect(run('')).toEqual([])
  })

  it('标题子串，大小写不敏感', () => {
    expect(run('LOGIN RED')).toEqual([0])
  })

  it('作者', () => {
    expect(run('smith')).toEqual([0])
  })

  it('分支名和 tag 名', () => {
    expect(run('feature/')).toEqual([1])
    expect(run('v1.0')).toEqual([0])
  })

  it('多个词要命中同一条提交', () => {
    // login 两条都有（标题 / 分支名），bob 只有第一条
    expect(run('login')).toEqual([0, 1])
    expect(run('login bob')).toEqual([0])
    expect(run('search bob')).toEqual([])
  })

  it('词不跨字段拼接：标题末尾 + 作者开头不算命中', () => {
    // 第一条标题以 redirect 结尾、作者以 Bob 开头
    expect(run('redirectbob')).toEqual([])
    expect(run('redirect\nbob')).toEqual([0]) // 换行被当成空白拆成两个词
  })

  it('hash 只认前缀，且至少 4 位', () => {
    expect(run('a1b2')).toEqual([0])
    expect(run(HASH_A.toUpperCase())).toEqual([0])
    // 3 位不参与 hash 匹配：输入 add / a1b 这种不该命中一堆 hash
    expect(run('a1b')).toEqual([])
    // 中间的子串不算
    expect(run('c3d4e5')).toEqual([])
  })

  it('命中按图上的行序返回', () => {
    const many = [commit('x0', 'wip'), commit('x1', 'done'), commit('x2', 'wip again')]
    expect(findMatches(buildSearchIndex(many, []), ['wip'])).toEqual([0, 2])
  })
})

describe('findMatches：SVN 节点', () => {
  const delivered = commit(HASH_A, 'local tweak', { author: 'Carol' })
  const commits = [
    commit('svn:1234', 'server change', { vcs: 'svn', svnRev: 1234, deliveredFrom: [delivered] }),
    commit('svn:99', 'old', { vcs: 'svn', svnRev: 99 }),
  ]
  const index = buildSearchIndex(commits, [])
  const run = (q: string) => findMatches(index, parseSearch(q))

  it('r1234 与 1234 都认 revision', () => {
    expect(run('r1234')).toEqual([0])
    expect(run('1234')).toEqual([0])
    expect(run('R99')).toEqual([1])
  })

  it('revision 要整个相等：r12 不命中 r1234', () => {
    expect(run('r12')).toEqual([])
  })

  it('收在节点里的已交付本地提交：标题、作者、hash 都算这个节点的', () => {
    expect(run('tweak')).toEqual([0])
    expect(run('carol')).toEqual([0])
    expect(run('a1b2c3')).toEqual([0])
  })

  it('svn 节点的 id（svn:1234）不当 hash 用', () => {
    expect(run('svn:')).toEqual([])
  })
})

describe('stepMatch', () => {
  const matches = [2, 5, 9]

  it('没有命中：null', () => {
    expect(stepMatch([], 3, 1, false)).toBeNull()
  })

  it('往下：当前之后的第一个；到底绕回第一个', () => {
    expect(stepMatch(matches, 2, 1, false)).toBe(5)
    expect(stepMatch(matches, 6, 1, false)).toBe(9)
    expect(stepMatch(matches, 9, 1, false)).toBe(2)
  })

  it('往上：当前之前的最后一个；到顶绕回最后一个', () => {
    expect(stepMatch(matches, 5, -1, false)).toBe(2)
    expect(stepMatch(matches, 2, -1, false)).toBe(9)
  })

  it('inclusive：当前本身就是命中时留在原地（输入时不让选中乱跳）', () => {
    expect(stepMatch(matches, 5, 1, true)).toBe(5)
    expect(stepMatch(matches, 4, 1, true)).toBe(5)
  })

  it('没有选中（-1）：往下取第一个，往上取最后一个', () => {
    expect(stepMatch(matches, -1, 1, false)).toBe(2)
    expect(stepMatch(matches, -1, -1, false)).toBe(9)
  })
})

const press = (key: string, extra: Partial<SearchKeyEvent> = {}): SearchKeyEvent => ({
  key, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...extra,
})

describe('windowSearchKey（窗口级快捷键）', () => {
  it('Ctrl+F / Cmd+F 聚焦', () => {
    expect(windowSearchKey(press('f', { ctrlKey: true }), false)).toBe('focus')
    expect(windowSearchKey(press('F', { metaKey: true }), false)).toBe('focus')
  })

  it('俄文等布局下 key 是本地字母，退回看物理键位；AZERTY 上别的拉丁字母不算', () => {
    expect(windowSearchKey(press('а', { ctrlKey: true, code: 'KeyF' }), false)).toBe('focus')
    expect(windowSearchKey(press('q', { ctrlKey: true, code: 'KeyF' }), false)).toBeNull()
  })

  it('Ctrl+Shift+F、Ctrl+Alt+F 留给别的用途', () => {
    expect(windowSearchKey(press('F', { ctrlKey: true, shiftKey: true }), false)).toBeNull()
    expect(windowSearchKey(press('f', { ctrlKey: true, altKey: true }), false)).toBeNull()
  })

  it('F3 / Shift+F3 只在搜着时才算', () => {
    expect(windowSearchKey(press('F3'), true)).toBe('next')
    expect(windowSearchKey(press('F3', { shiftKey: true }), true)).toBe('prev')
    expect(windowSearchKey(press('F3'), false)).toBeNull()
  })

  it('按住 F3 的自动重复不算：每跳一下都要读一次 diff', () => {
    expect(windowSearchKey(press('F3', { repeat: true }), true)).toBeNull()
  })

  it('输入法组词时一律不管', () => {
    expect(windowSearchKey(press('f', { ctrlKey: true, isComposing: true }), true)).toBeNull()
    expect(windowSearchKey(press('F3', { isComposing: true }), true)).toBeNull()
  })
})

describe('inputSearchKey（搜索框里）', () => {
  it('Enter 下一个，Shift+Enter 上一个，Esc', () => {
    expect(inputSearchKey(press('Enter'))).toBe('next')
    expect(inputSearchKey(press('Enter', { shiftKey: true }))).toBe('prev')
    expect(inputSearchKey(press('Escape'))).toBe('escape')
  })

  it('输入法组词时 Enter 是上屏、Esc 是取消候选，都不归搜索', () => {
    expect(inputSearchKey(press('Enter', { isComposing: true }))).toBeNull()
    expect(inputSearchKey(press('Escape', { isComposing: true }))).toBeNull()
  })

  it('按住 Enter 的自动重复不算', () => {
    expect(inputSearchKey(press('Enter', { repeat: true }))).toBeNull()
  })

  it('Ctrl+Enter 之类的组合不占', () => {
    expect(inputSearchKey(press('Enter', { ctrlKey: true }))).toBeNull()
  })
})

describe('matchPosition', () => {
  it('当前是第几个命中（从 1 起）；不是命中时为 0', () => {
    expect(matchPosition([2, 5, 9], 5)).toBe(2)
    expect(matchPosition([2, 5, 9], 4)).toBe(0)
    expect(matchPosition([], -1)).toBe(0)
  })
})
