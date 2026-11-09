import { describe, expect, it } from 'vitest'
import { countChanges, diffSequences, ignoreEolChanges, stripCr } from '@/engine/diffEol'
import type { DiffHunk, DiffLine } from '@/engine/types'

const ctx = (content: string, o: number, n: number): DiffLine => ({ origin: ' ', content, oldLineno: o, newLineno: n })
const del = (content: string, o: number): DiffLine => ({ origin: '-', content, oldLineno: o, newLineno: null })
const add = (content: string, n: number): DiffLine => ({ origin: '+', content, oldLineno: null, newLineno: n })

/** libgit2 的真实形态：LF → CRLF 时整块先删后增，新版每行带 `\r`（tests/diff_view_options.rs 钉住）。 */
function crlfHunk(texts: string[], changedAt: number | null = null): DiffHunk {
  const lines: DiffLine[] = [
    ...texts.map((t, i) => del(t, i + 1)),
    ...texts.map((t, i) => add(`${i === changedAt ? `${t} changed` : t}\r`, i + 1)),
  ]
  return { oldStart: 1, newStart: 1, header: `@@ -1,${texts.length} +1,${texts.length} @@`, lines }
}

const lines = (n: number) => Array.from({ length: n }, (_, i) => `line ${i + 1}`)

describe('stripCr', () => {
  it('只去掉行尾的一个 \\r', () => {
    expect(stripCr('a\r')).toBe('a')
    expect(stripCr('a\r\r')).toBe('a\r')
    expect(stripCr('a\rb')).toBe('a\rb')
    expect(stripCr('')).toBe('')
  })
})

describe('diffSequences', () => {
  const apply = (a: string[], b: string[]) => {
    const ops = diffSequences(a, b)!
    // 按操作序列重建两侧，必须分别等于原文
    const ra: string[] = []
    const rb: string[] = []
    for (const op of ops) {
      if (op.kind !== '+') ra.push(a[op.a])
      if (op.kind !== '-') rb.push(b[op.b])
    }
    return { ops, ra, rb }
  }

  it('相同序列全是相等', () => {
    const { ops } = apply(['a', 'b'], ['a', 'b'])
    expect(ops.every((o) => o.kind === '=')).toBe(true)
  })

  it('空输入', () => {
    expect(diffSequences([], [])).toEqual([])
    expect(diffSequences([], ['x'])).toEqual([{ kind: '+', b: 0 }])
    expect(diffSequences(['x'], [])).toEqual([{ kind: '-', a: 0 }])
  })

  it('中间插入、删除、替换都能还原两侧，且改动数最少', () => {
    const a = ['a', 'b', 'c', 'd', 'e']
    const b = ['a', 'x', 'c', 'e', 'f']
    const { ops, ra, rb } = apply(a, b)
    expect(ra).toEqual(a)
    expect(rb).toEqual(b)
    // b→x 一删一增、d 删、f 增
    expect(ops.filter((o) => o.kind !== '=')).toHaveLength(4)
  })

  it('改动超过上限时放弃', () => {
    const a = Array.from({ length: 20 }, (_, i) => `a${i}`)
    const b = Array.from({ length: 20 }, (_, i) => `b${i}`)
    expect(diffSequences(a, b, 10)).toBeNull()
  })

  it('两端相同的大段不算进改动数：一万行只改一行也能做', () => {
    const a = lines(10000)
    const b = [...a]
    b[5000] = 'changed'
    const ops = diffSequences(a, b, 5)!
    expect(ops.filter((o) => o.kind !== '=')).toHaveLength(2)
  })
})

describe('ignoreEolChanges', () => {
  it('整个文件只是 LF → CRLF：hunk 全部藏掉，并报告藏了东西', () => {
    const r = ignoreEolChanges([crlfHunk(lines(5))], 3)
    expect(r.hunks).toEqual([])
    expect(r.hidden).toBe(true)
  })

  it('换行符全变了、中间真改了一行：只剩那一行的一删一增和上下文', () => {
    const r = ignoreEolChanges([crlfHunk(lines(20), 9)], 3)
    expect(r.hunks).toHaveLength(1)
    const h = r.hunks[0]
    expect(h.lines.map((l) => l.origin).join('')).toBe('   -+   ')
    expect(h.lines[3]).toEqual(del('line 10', 10))
    expect(h.lines[4]).toEqual(add('line 10 changed\r', 10))
    // 配上的上下文行两个行号都有
    expect(h.lines[0]).toEqual(ctx('line 7\r', 7, 7))
    expect(h.header).toBe('@@ -7,7 +7,7 @@')
    expect([h.oldStart, h.newStart]).toEqual([7, 7])
  })

  it('上下文行数跟着设置走；离得近的两处改动并成一个 hunk，远的分开', () => {
    const texts = lines(30)
    const hunk: DiffHunk = {
      oldStart: 1, newStart: 1, header: '@@ -1,30 +1,30 @@ fn main',
      lines: [...texts.map((t, i) => del(t, i + 1)), ...texts.map((t, i) => add(`${[4, 8, 25].includes(i) ? 'X' : t}\r`, i + 1))],
    }
    const r = ignoreEolChanges([hunk], 2)
    // 第 5、9 行之间隔 3 行（≤ 2×2），并成一个；第 26 行另起一个
    expect(r.hunks.map((h) => h.header)).toEqual(['@@ -3,9 +3,9 @@', '@@ -24,5 +24,5 @@'])
    expect(ignoreEolChanges([hunk], 0).hunks).toHaveLength(3)
  })

  it('整个文件（上下文无限）时不切：一个 hunk，保留全部行', () => {
    const r = ignoreEolChanges([crlfHunk(lines(20), 9)], Number.POSITIVE_INFINITY)
    expect(r.hunks).toHaveLength(1)
    expect(r.hunks[0].lines).toHaveLength(21)
  })

  it('保留函数上下文：从原 hunk 开头切出来的那一段沿用头里 @@ 之后的部分', () => {
    const hunk: DiffHunk = {
      oldStart: 1, newStart: 1, header: '@@ -1,2 +1,2 @@ fn main',
      lines: [del('a', 1), del('b', 2), add('a\r', 1), add('B\r', 2)],
    }
    expect(ignoreEolChanges([hunk], 3).hunks[0].header).toBe('@@ -1,2 +1,2 @@ fn main')
  })

  it('没有只差换行符的改动时原样返回（同一个对象）', () => {
    const hunk: DiffHunk = {
      oldStart: 1, newStart: 1, header: '@@ -1,2 +1,2 @@',
      lines: [ctx('a', 1, 1), del('b', 2), add('c', 2)],
    }
    const r = ignoreEolChanges([hunk], 3)
    expect(r.hunks[0]).toBe(hunk)
    expect(r.hidden).toBe(false)
  })

  it('CRLF → LF 反方向同样认得', () => {
    const hunk: DiffHunk = {
      oldStart: 1, newStart: 1, header: '@@ -1,2 +1,2 @@',
      lines: [del('a\r', 1), del('b\r', 2), add('a', 1), add('b', 2)],
    }
    expect(ignoreEolChanges([hunk], 3).hunks).toEqual([])
  })

  it('行尾空格的改动不是换行符改动，照样显示', () => {
    const hunk: DiffHunk = {
      oldStart: 1, newStart: 1, header: '@@ -1 +1 @@',
      lines: [del('a', 1), add('a ', 1)],
    }
    const r = ignoreEolChanges([hunk], 3)
    expect(r.hunks).toEqual([hunk])
    expect(r.hidden).toBe(false)
  })

  it('纯新增、上下文为 0 时起始旧行号沿用前一行（同 git）', () => {
    const hunk: DiffHunk = {
      oldStart: 1, newStart: 1, header: '@@ -1,2 +1,3 @@',
      lines: [del('a', 1), del('b', 2), add('a\r', 1), add('new\r', 2), add('b\r', 3)],
    }
    const h = ignoreEolChanges([hunk], 0).hunks
    expect(h).toHaveLength(1)
    expect(h[0].lines).toEqual([add('new\r', 2)])
    expect(h[0].header).toBe('@@ -1,0 +2 @@')
  })

  it('文件末尾补上换行（两边行内容一模一样）不是换行符改动，照样显示', () => {
    // libgit2 去掉 `\n`、滤掉 `\ No newline` 标记之后，补换行就是内容相同的一删一增
    const hunk: DiffHunk = {
      oldStart: 1, newStart: 1, header: '@@ -1,2 +1,2 @@',
      lines: [ctx('a', 1, 1), del('b', 2), add('b', 2)],
    }
    const r = ignoreEolChanges([hunk], 3)
    expect(r.hunks).toEqual([hunk])
    expect(r.hidden).toBe(false)
  })

  it('同一块里既有补换行又有 CRLF：只藏 CRLF 那几行', () => {
    const hunk: DiffHunk = {
      oldStart: 1, newStart: 1, header: '@@ -1,2 +1,2 @@',
      lines: [del('a', 1), del('b', 2), add('a\r', 1), add('b', 2)],
    }
    const r = ignoreEolChanges([hunk], 3)
    expect(r.hidden).toBe(true)
    expect(r.hunks[0].lines).toEqual([ctx('a\r', 1, 1), del('b', 2), add('b', 2)])
  })

  it('空 hunk 列表', () => {
    expect(ignoreEolChanges([], 3)).toEqual({ hunks: [], hidden: false })
  })
})

describe('countChanges', () => {
  it('忽略换行符时只差 \\r 的不算，真改的照算', () => {
    const hunk = crlfHunk(lines(2000), 5)
    expect(countChanges([hunk], true)).toEqual({ additions: 1, deletions: 1 })
    expect(countChanges([hunk], false)).toEqual({ additions: 2000, deletions: 2000 })
  })

  it('只换了换行符：+0/-0', () => {
    expect(countChanges([crlfHunk(lines(10))], true)).toEqual({ additions: 0, deletions: 0 })
  })

  it('空输入', () => {
    expect(countChanges([], true)).toEqual({ additions: 0, deletions: 0 })
  })
})
