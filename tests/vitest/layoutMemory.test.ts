import { describe, expect, it } from 'vitest'
import {
  parseSectionMemory,
  sanitizePanelLayout,
  sectionOpen,
  withSectionOpen,
} from '@/engine/layoutMemory'

describe('parseSectionMemory', () => {
  it('没有存档时是空记忆', () => {
    expect(parseSectionMemory(null)).toEqual({})
  })

  it('坏 JSON、非对象、数组都退化成空记忆，不抛', () => {
    expect(parseSectionMemory('{')).toEqual({})
    expect(parseSectionMemory('"tags"')).toEqual({})
    expect(parseSectionMemory('null')).toEqual({})
    expect(parseSectionMemory('[true]')).toEqual({})
  })

  it('只留布尔值的条目', () => {
    expect(parseSectionMemory('{"tags":true,"stashes":"yes","branches":false,"worktrees":1}'))
      .toEqual({ tags: true, branches: false })
  })
})

describe('sectionOpen / withSectionOpen', () => {
  it('没记过用默认值，记过用记忆——包括记的是 false', () => {
    expect(sectionOpen({}, 'tags', false)).toBe(false)
    expect(sectionOpen({ tags: true }, 'tags', false)).toBe(true)
    expect(sectionOpen({ branches: false }, 'branches', true)).toBe(false)
  })

  it('写入一个分区不影响其他分区，也不改入参', () => {
    const before = { branches: false }
    const after = withSectionOpen(before, 'tags', true)
    expect(after).toEqual({ branches: false, tags: true })
    expect(before).toEqual({ branches: false })
  })
})

describe('sanitizePanelLayout', () => {
  it('合法布局原样返回', () => {
    const raw = '{"sidebar":22,"main":78}'
    expect(sanitizePanelLayout(raw)).toBe(raw)
  })

  it('没有存档、坏 JSON、空对象、数组都当作没有存档', () => {
    expect(sanitizePanelLayout(null)).toBeNull()
    expect(sanitizePanelLayout('{"sidebar":')).toBeNull()
    expect(sanitizePanelLayout('{}')).toBeNull()
    expect(sanitizePanelLayout('[22,78]')).toBeNull()
    expect(sanitizePanelLayout('null')).toBeNull()
  })

  it('值不是非负有限数的布局整个作废', () => {
    expect(sanitizePanelLayout('{"sidebar":"22","main":78}')).toBeNull()
    expect(sanitizePanelLayout('{"sidebar":-1,"main":101}')).toBeNull()
    expect(sanitizePanelLayout('{"sidebar":null,"main":78}')).toBeNull()
  })
})
