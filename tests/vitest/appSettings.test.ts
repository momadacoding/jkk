import { describe, expect, it } from 'vitest'
import { captureShortcut, displayShortcut, type KeyLike } from '@/engine/appSettings'

const key = (code: string, mods: Partial<KeyLike> = {}): KeyLike => ({
  code,
  key: '',
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
})

describe('captureShortcut', () => {
  it('把 Ctrl+Alt+字母 转成后端认的写法', () => {
    expect(captureShortcut(key('KeyJ', { ctrlKey: true, altKey: true }))).toEqual({
      kind: 'ok',
      accelerator: 'Ctrl+Alt+J',
    })
  })

  it('修饰键按固定顺序输出，与按下顺序无关', () => {
    expect(
      captureShortcut(key('F12', { metaKey: true, shiftKey: true, altKey: true, ctrlKey: true })),
    ).toEqual({ kind: 'ok', accelerator: 'Ctrl+Alt+Shift+Super+F12' })
  })

  it('按 code 而不是 key 取键：Shift 把 1 变成 ! 时也认成 Digit1', () => {
    expect(captureShortcut({ ...key('Digit1', { ctrlKey: true, shiftKey: true }), key: '!' })).toEqual({
      kind: 'ok',
      accelerator: 'Ctrl+Shift+1',
    })
  })

  it('标点、方向键、功能键用解析器认得的名字', () => {
    const ctrl = { ctrlKey: true }
    expect(captureShortcut(key('Backquote', ctrl))).toEqual({ kind: 'ok', accelerator: 'Ctrl+Backquote' })
    expect(captureShortcut(key('ArrowUp', ctrl))).toEqual({ kind: 'ok', accelerator: 'Ctrl+ArrowUp' })
    expect(captureShortcut(key('Space', ctrl))).toEqual({ kind: 'ok', accelerator: 'Ctrl+Space' })
    expect(captureShortcut(key('Numpad5', ctrl))).toEqual({ kind: 'ok', accelerator: 'Ctrl+Numpad5' })
  })

  it('只按了修饰键时继续等主键', () => {
    for (const code of ['ControlLeft', 'AltRight', 'ShiftLeft', 'MetaLeft']) {
      expect(captureShortcut(key(code, { ctrlKey: true }))).toEqual({ kind: 'pending' })
    }
  })

  it('没有 Ctrl / Alt / Win 的组合拒收：全局注册后别的程序里就打不出这个字', () => {
    expect(captureShortcut(key('KeyJ')).kind).toBe('invalid')
    expect(captureShortcut(key('KeyJ', { shiftKey: true })).kind).toBe('invalid')
    expect(captureShortcut(key('F5')).kind).toBe('invalid')
  })

  it('按着 AltGr 时拒收：注册成 Ctrl+Alt+X 会吞掉欧洲布局的 AltGr 字符', () => {
    expect(captureShortcut(key('KeyQ', { ctrlKey: true, altKey: true, altGraph: true })).kind).toBe(
      'invalid',
    )
  })

  it('不收 Tab：弹框的焦点圈会先把它拿走', () => {
    expect(captureShortcut(key('Tab', { ctrlKey: true })).kind).toBe('invalid')
  })

  it('不认识的键拒收，而不是拼出一个后端解析不了的字符串', () => {
    expect(captureShortcut(key('IntlRo', { ctrlKey: true })).kind).toBe('invalid')
    expect(captureShortcut(key('', { ctrlKey: true })).kind).toBe('invalid')
  })
})

describe('displayShortcut', () => {
  it('Super 在界面上显示成 Win，空值显示为未设置', () => {
    expect(displayShortcut('Ctrl+Super+J')).toBe('Ctrl+Win+J')
    expect(displayShortcut(null)).toBe('未设置')
  })
})
