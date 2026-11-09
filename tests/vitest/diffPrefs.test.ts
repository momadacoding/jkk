import { describe, expect, it } from 'vitest'
import {
  CONTEXT_STEPS,
  DEFAULT_DIFF_PREFS,
  FULL_FILE_CONTEXT,
  backendDiffOptions,
  diffOptionsKey,
  effectiveContext,
  parseDiffPrefs,
  stepContext,
} from '@/engine/diffPrefs'

describe('DEFAULT_DIFF_PREFS', () => {
  it('默认忽略换行符，其余与 git 默认一致', () => {
    expect(DEFAULT_DIFF_PREFS).toEqual({
      split: false, ignoreEol: true, ignoreWhitespace: false, showInvisibles: false,
      wrap: false, contextLines: 3, fullFile: false,
    })
  })
})

describe('parseDiffPrefs', () => {
  it('没有存档、坏 JSON、不是对象都回到默认', () => {
    for (const raw of [null, '', '{', 'null', '42', '"x"']) {
      expect(parseDiffPrefs(raw)).toEqual(DEFAULT_DIFF_PREFS)
    }
  })

  it('字段逐个校验，类型不对的回到默认，其余保留', () => {
    const p = parseDiffPrefs(JSON.stringify({ split: true, ignoreEol: 'no', contextLines: 7, wrap: true }))
    expect(p).toEqual({ ...DEFAULT_DIFF_PREFS, split: true, wrap: true })
  })

  it('档位上的上下文行数保留', () => {
    expect(parseDiffPrefs(JSON.stringify({ contextLines: 10 })).contextLines).toBe(10)
  })

  it('返回的是副本，改它不会改到默认值', () => {
    const p = parseDiffPrefs(null)
    p.split = true
    expect(DEFAULT_DIFF_PREFS.split).toBe(false)
  })
})

describe('stepContext', () => {
  it('按档位走，到头返回 null', () => {
    expect(stepContext(3, 1)).toBe(5)
    expect(stepContext(3, -1)).toBe(1)
    expect(stepContext(0, -1)).toBeNull()
    expect(stepContext(CONTEXT_STEPS[CONTEXT_STEPS.length - 1], 1)).toBeNull()
  })

  it('不在档位上的值走到最近的档位', () => {
    expect(stepContext(4, 1)).toBe(5)
    expect(stepContext(4, -1)).toBe(3)
  })
})

describe('backendDiffOptions', () => {
  it('只带上下文行数和忽略空白；整个文件时上下文用上限', () => {
    expect(backendDiffOptions(DEFAULT_DIFF_PREFS)).toEqual({ contextLines: 3, ignoreWhitespace: false })
    expect(backendDiffOptions({ ...DEFAULT_DIFF_PREFS, fullFile: true, ignoreWhitespace: true }))
      .toEqual({ contextLines: FULL_FILE_CONTEXT, ignoreWhitespace: true })
  })

  it('纯画法的开关不改变键：切换它们不会重读', () => {
    const base = diffOptionsKey(backendDiffOptions(DEFAULT_DIFF_PREFS))
    const drawn = { ...DEFAULT_DIFF_PREFS, split: true, wrap: true, showInvisibles: true, ignoreEol: false }
    expect(diffOptionsKey(backendDiffOptions(drawn))).toBe(base)
    expect(diffOptionsKey(backendDiffOptions({ ...DEFAULT_DIFF_PREFS, contextLines: 5 }))).not.toBe(base)
    expect(diffOptionsKey(backendDiffOptions({ ...DEFAULT_DIFF_PREFS, ignoreWhitespace: true }))).not.toBe(base)
  })
})

describe('effectiveContext', () => {
  it('整个文件时不限', () => {
    expect(effectiveContext(DEFAULT_DIFF_PREFS)).toBe(3)
    expect(effectiveContext({ ...DEFAULT_DIFF_PREFS, fullFile: true })).toBe(Number.POSITIVE_INFINITY)
  })
})
