/**
 * 应用级设置（关窗进托盘、全局快捷键、SVN 自动跟进）的类型，与设置框里录制快捷键的纯逻辑。
 */

/** 后端 `read_app_settings` / `save_app_settings` 的返回值。 */
export interface AppSettingsView {
  closeToTray: boolean
  /** 托盘图标没建出来：关窗只能直接退出，「隐藏到托盘」那一项要置灰。 */
  trayAvailable: boolean
  /** `null` 表示快捷键已关闭。 */
  showShortcut: string | null
  /** 已保存的快捷键此刻没注册上（被别的程序占了等）的原因。 */
  shortcutError: string | null
  defaultShortcut: string
  /** SVN 混合仓库里自动跟进 TortoiseSVN 的提交 / 更新（事故分析 6.12）。 */
  svnAutoFollow: boolean
}

/** 录制只用到 KeyboardEvent 的这几个字段，单测不必造真事件。 */
export interface KeyLike {
  code: string
  key: string
  ctrlKey: boolean
  altKey: boolean
  shiftKey: boolean
  metaKey: boolean
  /**
   * 按着 AltGr。Windows 上 AltGr 会同时报 ctrlKey 和 altKey，不单独看它会录成 Ctrl+Alt+X，
   * 注册成全局快捷键后欧洲键盘布局上的 AltGr 字符就打不出来了。
   */
  altGraph?: boolean
}

export type ShortcutCapture =
  | { kind: 'ok'; accelerator: string }
  /** 只按了修饰键，继续等主键。 */
  | { kind: 'pending' }
  | { kind: 'invalid'; reason: string }

const MODIFIER_CODES = new Set([
  'ControlLeft',
  'ControlRight',
  'AltLeft',
  'AltRight',
  'ShiftLeft',
  'ShiftRight',
  'MetaLeft',
  'MetaRight',
  'OSLeft',
  'OSRight',
])

/**
 * 原样交给后端的 `KeyboardEvent.code`。名字与 global-hotkey 的解析器（`parse_key`）一致，
 * 解析器不区分大小写，但这里保持原样，存进 settings.json 也好读。
 */
const PASS_THROUGH = new Set([
  // 不收 Tab：弹框的焦点圈在捕获阶段就把 Tab 拿走了，录制框收不到
  'Space', 'Enter', 'Backspace', 'Delete', 'Insert', 'Home', 'End', 'PageUp', 'PageDown',
  'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight',
  'Backquote', 'Backslash', 'BracketLeft', 'BracketRight', 'Comma', 'Equal', 'Minus', 'Period',
  'Quote', 'Semicolon', 'Slash',
  'NumpadAdd', 'NumpadSubtract', 'NumpadMultiply', 'NumpadDivide', 'NumpadDecimal',
])

/** `KeyboardEvent.code` → 后端认得的主键名；不认识的返回 `null`。 */
function keyName(code: string): string | null {
  // 用 code 而不是 key：Shift 会把 key 从 1 变成 !，键盘布局也会改 key，code 只看物理位置
  const letter = /^Key([A-Z])$/.exec(code)
  if (letter) return letter[1]
  const digit = /^Digit([0-9])$/.exec(code)
  if (digit) return digit[1]
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code
  if (/^Numpad[0-9]$/.test(code)) return code
  return PASS_THROUGH.has(code) ? code : null
}

/**
 * 把录制框里的一次按键转成快捷键字符串（与后端 `validate_shortcut` 同一套规则）。
 *
 * 修饰键按固定顺序输出：同一个组合不管先按哪个键，存下来都是同一个字符串。
 */
export function captureShortcut(e: KeyLike): ShortcutCapture {
  if (MODIFIER_CODES.has(e.code)) return { kind: 'pending' }
  if (e.altGraph) return { kind: 'invalid', reason: '不能用 AltGr，请用左 Ctrl + 左 Alt' }
  const name = keyName(e.code)
  if (!name) return { kind: 'invalid', reason: '不支持这个键，换一个试试' }
  if (!e.ctrlKey && !e.altKey && !e.metaKey) {
    return { kind: 'invalid', reason: '至少要带 Ctrl、Alt 或 Win 中的一个' }
  }
  const parts: string[] = []
  if (e.ctrlKey) parts.push('Ctrl')
  if (e.altKey) parts.push('Alt')
  if (e.shiftKey) parts.push('Shift')
  if (e.metaKey) parts.push('Super')
  parts.push(name)
  return { kind: 'ok', accelerator: parts.join('+') }
}

/** 界面上显示的写法：解析器里的 Super 就是 Windows 键。 */
export function displayShortcut(accelerator: string | null): string {
  if (!accelerator) return '未设置'
  return accelerator
    .split('+')
    .map((p) => (p.toLowerCase() === 'super' ? 'Win' : p))
    .join('+')
}
