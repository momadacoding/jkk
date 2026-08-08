import { describe, expect, it } from 'vitest'
import { commitKeyAction, type CommitKeyEvent } from '../../src/engine/commitKeys'

/** 默认是「什么修饰键都没按的 Enter」，每条用例只改自己关心的那一两个字段。 */
function key(over: Partial<CommitKeyEvent> = {}): CommitKeyEvent {
  return { key: 'Enter', ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...over }
}

describe('commitKeyAction', () => {
  describe('subject 输入框', () => {
    it('Enter 跳到 description，而不是提交', () => {
      expect(commitKeyAction('subject', key(), true)).toBe('focus-description')
    })

    it('subject 还是空的也照样跳——光标移动不该有前置条件', () => {
      expect(commitKeyAction('subject', key(), false)).toBe('focus-description')
    })

    it('Ctrl+Enter 提交', () => {
      expect(commitKeyAction('subject', key({ ctrlKey: true }), true)).toBe('commit')
    })

    it('Cmd+Enter 也提交（macOS）', () => {
      expect(commitKeyAction('subject', key({ metaKey: true }), true)).toBe('commit')
    })

    it('Shift+Enter 什么都不做，不要在 subject 里跳走', () => {
      expect(commitKeyAction('subject', key({ shiftKey: true }), true)).toBe('none')
    })

    it('Alt+Enter 什么都不做', () => {
      expect(commitKeyAction('subject', key({ altKey: true }), true)).toBe('none')
    })
  })

  describe('description 文本域', () => {
    it('Enter 保持换行的默认行为，不拦', () => {
      expect(commitKeyAction('description', key(), true)).toBe('none')
    })

    it('Ctrl+Enter 提交', () => {
      expect(commitKeyAction('description', key({ ctrlKey: true }), true)).toBe('commit')
    })

    it('Cmd+Enter 也提交', () => {
      expect(commitKeyAction('description', key({ metaKey: true }), true)).toBe('commit')
    })
  })

  describe('不能提交的时候', () => {
    it('subject 为空时 Ctrl+Enter 不提交——和按钮 disabled 保持一致', () => {
      expect(commitKeyAction('subject', key({ ctrlKey: true }), false)).toBe('none')
      expect(commitKeyAction('description', key({ ctrlKey: true }), false)).toBe('none')
    })
  })

  describe('输入法组词', () => {
    // 中文/日文输入法用 Enter 上屏候选词。这一下要是被当成提交或跳转，
    // 用户每打一个中文字都可能被弹走，属于必须挡住的误触。
    it('组词中的 Enter 一律不响应', () => {
      expect(commitKeyAction('subject', key({ isComposing: true }), true)).toBe('none')
      expect(commitKeyAction('description', key({ isComposing: true }), true)).toBe('none')
    })

    it('组词中的 Ctrl+Enter 也不提交', () => {
      expect(commitKeyAction('subject', key({ ctrlKey: true, isComposing: true }), true)).toBe('none')
    })
  })

  describe('其他按键', () => {
    it('不是 Enter 就一律不管', () => {
      expect(commitKeyAction('subject', key({ key: 'a' }), true)).toBe('none')
      expect(commitKeyAction('subject', key({ key: 'Tab' }), true)).toBe('none')
      expect(commitKeyAction('description', key({ key: 'Escape', ctrlKey: true }), true)).toBe('none')
    })
  })
})
