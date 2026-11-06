import { describe, expect, it } from 'vitest'
import { applyStatusPatch, gitPathCompare, type StatusPatch } from '@/engine/statusPatch'
import type { FileChange, StatusData } from '@/engine/types'

const m = (path: string, additions = 1): FileChange => ({ path, status: 'modified', additions, deletions: 0 })
const a = (path: string): FileChange => ({ path, status: 'added', additions: 1, deletions: 0 })
const d = (path: string): FileChange => ({ path, status: 'deleted', additions: 0, deletions: 1 })
const paths = (files: FileChange[]) => files.map((f) => f.path)

function patch(staged: FileChange[], unstaged: FileChange[], ignoreCase = true): StatusPatch {
  return { staged, unstaged, ignoreCase }
}

describe('applyStatusPatch：Stage / Unstage 之后就地更新列表', () => {
  it('Stage 一个文件：它从 Unstaged 消失，Staged 换成重读的那份', () => {
    const prev: StatusData = { staged: [], unstaged: [m('a.txt'), m('b.txt'), m('c.txt')] }
    const next = applyStatusPatch(prev, ['b.txt'], patch([m('b.txt')], []))
    expect(next).toEqual({ staged: [m('b.txt')], unstaged: [m('a.txt'), m('c.txt')] })
  })

  it('Unstage 一个文件：它按 git 的顺序插回 Unstaged，不是追加到末尾', () => {
    const prev: StatusData = { staged: [m('b.txt')], unstaged: [m('a.txt'), m('c.txt')] }
    const next = applyStatusPatch(prev, ['b.txt'], patch([], [m('b.txt')]))
    expect(next).toEqual({ staged: [], unstaged: [m('a.txt'), m('b.txt'), m('c.txt')] })
  })

  it('点名的路径原来就在：原地换成新的那一项（行数变了）', () => {
    const prev: StatusData = { staged: [], unstaged: [m('a.txt'), m('b.txt', 1), m('c.txt')] }
    const next = applyStatusPatch(prev, ['b.txt'], patch([], [m('b.txt', 5)]))
    expect(next?.unstaged).toEqual([m('a.txt'), m('b.txt', 5), m('c.txt')])
  })

  it('一次 Stage 多个、Unstage 多个：几项一起换，顺序仍按 git 排', () => {
    const prev: StatusData = { staged: [m('b.txt'), m('d.txt')], unstaged: [m('a.txt'), m('c.txt'), m('e.txt')] }
    const next = applyStatusPatch(prev, ['b.txt', 'd.txt', 'e.txt'], patch([m('e.txt')], [m('b.txt'), m('d.txt')]))
    expect(next).toEqual({ staged: [m('e.txt')], unstaged: [m('a.txt'), m('b.txt'), m('c.txt'), m('d.txt')] })
  })

  // `git add t/inner.txt` 会把 index 里的文件 `t` 一起删掉（`status_paths_contract.rs` 实测）。
  // `t` 没被点名，局部重读看不到它在 Unstaged 那一项已经没了：只能退回全量重读
  it('没点名的路径在 Staged 里也变了（git 的连带改动）：交回 null，调用方全量重读', () => {
    const prev: StatusData = { staged: [], unstaged: [d('t'), a('t/inner.txt')] }
    const next = applyStatusPatch(prev, ['t/inner.txt'], patch([d('t'), a('t/inner.txt')], []))
    expect(next).toBeNull()
  })

  it('没点名的路径在 Staged 里行数变了，同样算连带改动', () => {
    const prev: StatusData = { staged: [m('x.txt', 1)], unstaged: [m('a.txt')] }
    expect(applyStatusPatch(prev, ['a.txt'], patch([m('a.txt'), m('x.txt', 2)], []))).toBeNull()
  })

  // 后端已经只留点名路径（libgit2 的字面量 pathspec 仍按目录前缀匹配，点名 `t` 会带出 `t/x`）。
  // 这里再挡一道：真带出来了就不合并，否则那一项会和列表里原有的各留一份
  it('重读结果里有没点名的路径：交回 null，不合并出重复项', () => {
    const prev: StatusData = { staged: [d('t')], unstaged: [a('t/x')] }
    expect(applyStatusPatch(prev, ['t'], patch([], [d('t'), a('t/x')]))).toBeNull()
  })

  it('没点名的路径在 Staged 里原样留着：不算连带改动', () => {
    const prev: StatusData = { staged: [m('x.txt')], unstaged: [m('a.txt')] }
    expect(applyStatusPatch(prev, ['a.txt'], patch([m('a.txt'), m('x.txt')], []))).toEqual({
      staged: [m('a.txt'), m('x.txt')],
      unstaged: [],
    })
  })

  it('点名的路径只有部分重读回来：其余的从 Unstaged 去掉（它们已经没有未暂存的改动了）', () => {
    const prev: StatusData = { staged: [], unstaged: [m('a.txt'), m('b.txt')] }
    const next = applyStatusPatch(prev, ['a.txt', 'b.txt'], patch([m('a.txt'), m('b.txt')], []))
    expect(next?.unstaged).toEqual([])
  })

  it('点名了一个不在任何列表里的路径：其余不动', () => {
    const prev: StatusData = { staged: [m('s.txt')], unstaged: [m('a.txt')] }
    expect(applyStatusPatch(prev, ['nope.txt'], patch([m('s.txt')], []))).toEqual(prev)
  })

  it('不改动传进来的旧状态', () => {
    const prev: StatusData = { staged: [], unstaged: [m('a.txt'), m('b.txt')] }
    const snapshot = structuredClone(prev)
    applyStatusPatch(prev, ['a.txt'], patch([m('a.txt')], []))
    expect(prev).toEqual(snapshot)
  })

  it('不区分大小写的 index：按不区分大小写的顺序插', () => {
    const prev: StatusData = { staged: [m('B.txt')], unstaged: [m('a.txt'), m('C.txt')] }
    const next = applyStatusPatch(prev, ['B.txt'], patch([], [m('B.txt')], true))
    expect(paths(next!.unstaged)).toEqual(['a.txt', 'B.txt', 'C.txt'])
  })

  it('区分大小写的 index：按字节顺序插（大写排在小写前）', () => {
    const prev: StatusData = { staged: [m('b.txt')], unstaged: [m('C.txt'), m('a.txt')] }
    const next = applyStatusPatch(prev, ['b.txt'], patch([], [m('b.txt')], false))
    expect(paths(next!.unstaged)).toEqual(['C.txt', 'a.txt', 'b.txt'])
  })
})

// libgit2 的 diff 按完整路径逐字节比（不区分大小写时只折叠 ASCII 字母），目录分隔符 `/` 也只是一个字节
describe('gitPathCompare：与 libgit2 列表的顺序一致', () => {
  const sorted = (list: string[], icase: boolean) => [...list].sort((x, y) => gitPathCompare(x, y, icase))

  it('`/` 按字节比：`a.txt` < `a/b` < `a0`，`a-b` < `a/x`', () => {
    expect(sorted(['a0', 'a/b', 'a.txt'], false)).toEqual(['a.txt', 'a/b', 'a0'])
    expect(sorted(['a/x', 'a-b'], false)).toEqual(['a-b', 'a/x'])
  })

  it('前缀排在前面', () => {
    expect(sorted(['ab', 'a'], false)).toEqual(['a', 'ab'])
  })

  it('非 ASCII 按码位比，与 UTF-8 字节序相同（BMP 以外的字符也一样）', () => {
    // U+FF21（全角 A）与 U+1F600（😀）：UTF-16 码元比会把后者排前面，UTF-8 字节序是前者在前
    expect(sorted(['😀.txt', 'Ａ.txt', '中.txt'], false)).toEqual(['中.txt', 'Ａ.txt', '😀.txt'])
  })

  it('不区分大小写只折叠 ASCII 字母', () => {
    expect(gitPathCompare('ABC', 'abc', true)).toBe(0)
    // `_` 是 0x5F：折叠成小写后比 `a`（0x61）小，区分大小写时比 `A`（0x41）大（libgit2 `git__strcasecmp`）
    expect(gitPathCompare('_x', 'Ax', true)).toBeLessThan(0)
    expect(gitPathCompare('_x', 'Ax', false)).toBeGreaterThan(0)
    // 非 ASCII 字母不折叠
    expect(gitPathCompare('Ä', 'ä', true)).not.toBe(0)
  })
})
