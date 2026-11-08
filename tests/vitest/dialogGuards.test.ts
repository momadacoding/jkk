/**
 * 弹框的结构约定。这里读源码，不渲染：jsdom 不做布局，真实布局由 `pnpm test:layout` 量；
 * 这个文件跑在每次 `pnpm test` 里，负责拦住「新增了弹框却没进布局测试」这类漏网。
 */
import { readdirSync, readFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { describe, expect, it } from 'vitest'
import { SCENARIOS } from '../layout/scenarios'

const ROOT = process.cwd()
const SRC = join(ROOT, 'src')

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const full = join(dir, e.name)
    if (e.isDirectory()) return sources(full)
    return /\.tsx?$/.test(e.name) ? [full] : []
  })
}

const rel = (file: string) => relative(ROOT, file).split('\\').join('/')

/** 取一条 CSS 规则的声明块。只认从行首开始的选择器，不会碰到 `.foo .dialog {`。 */
function cssRule(selector: string): string {
  const css = readFileSync(join(SRC, 'styles.css'), 'utf8')
  const at = css.search(new RegExp(`^${selector.replace('.', '\\.')} \\{`, 'm'))
  if (at < 0) throw new Error(`styles.css 里没有 ${selector}`)
  return css.slice(at, css.indexOf('}', at))
}

/**
 * 源码里每个带 `dialog-overlay` 的 JSX 开始标签（从 `<` 到收尾的 `>`）。
 *
 * 按花括号深度找收尾：属性里的箭头函数 `=>`、比较 `a > b` 都在 `{}` 里，不会被当成标签结束；
 * 不看属性顺序，也不管 className 是字面量还是模板字符串。
 */
function overlayTags(src: string): string[] {
  const tags: string[] = []
  let from = 0
  for (;;) {
    const at = src.indexOf('dialog-overlay', from)
    if (at < 0) return tags
    const start = src.lastIndexOf('<', at)
    let depth = 0
    let end = start
    for (; end < src.length; end++) {
      const ch = src[end]
      if (ch === '{') depth++
      else if (ch === '}') depth--
      else if (ch === '>' && depth === 0) break
    }
    tags.push(src.slice(start, end + 1))
    from = end + 1
  }
}

describe('弹框的结构约定', () => {
  it('每个弹框组件都在布局测试里有场景', () => {
    // 新增弹框走到这里失败时：在 tests/layout/scenarios.ts 和 harness.tsx 里加一个超长场景
    const dialogs = sources(join(SRC, 'components'))
      .filter((f) => readFileSync(f, 'utf8').includes('dialog-overlay'))
      .map((f) => f.split(/[\\/]/).pop()!.replace(/\.tsx$/, ''))
      .sort()
    const covered = [...new Set(SCENARIOS.map((s) => s.component))].sort()
    expect(covered).toEqual(dialogs)
  })

  it('每个场景在复现页里都有实现', () => {
    // 只加进 scenarios.ts 的话 pnpm test 照样绿，要等手动跑 test:layout 才炸
    const harness = readFileSync(join(ROOT, 'tests/layout/harness.tsx'), 'utf8')
    const missing = SCENARIOS.filter((s) => !harness.includes(`'${s.id}':`)).map((s) => s.id)
    expect(missing).toEqual([])
  })

  it('遮罩上不挂任何事件：点背景不关框，关框只走按钮和 Esc', () => {
    // 点背景就关，是确认框里点偏一下就丢了的根源（2026-11-08 丢弃确认框实测），
    // 有输入框的框（PromptDialog、DeliverDialog）还会连带丢掉已经打的字
    const files = sources(join(SRC, 'components')).filter((f) => readFileSync(f, 'utf8').includes('dialog-overlay'))
    expect(files.length).toBeGreaterThan(0)
    const offenders = files.flatMap((f) =>
      overlayTags(readFileSync(f, 'utf8'))
        .filter((tag) => /\son[A-Z]\w*\s*=/.test(tag))
        .map((tag) => `${rel(f)}: ${tag}`),
    )
    expect(offenders).toEqual([])
  })

  it('取遮罩开始标签的写法拦得住各种绕法（守卫自己的用例）', () => {
    const cases = [
      '<div className="dialog-overlay" onKeyDown={(e) => e.key} onMouseDown={close}>',
      '<div onMouseDown={close} className="dialog-overlay">',
      '<div className={`dialog-overlay ${x}`} onClick={() => a > b && close()}>',
      '<div\n  className="dialog-overlay"\n  onPointerDown={close}\n>',
    ]
    for (const src of cases) {
      const tags = overlayTags(src)
      expect(tags).toHaveLength(1)
      expect(tags[0]).toMatch(/\son[A-Z]\w*\s*=/)
    }
    expect(overlayTags('<div className="dialog-overlay">\n<div onMouseDown={x}>')).toEqual([
      '<div className="dialog-overlay">',
    ])
  })

  it('原生确认框只留给固定文案：动态清单要走应用内确认框', () => {
    // 原生框的布局不归我们管，布局测试也量不到它（Windows 上是 TaskDialog，长文案下的表现没实测过）
    const callers = sources(SRC)
      .filter((f) => !f.endsWith('confirm.ts'))
      .filter((f) => readFileSync(f, 'utf8').includes('askConfirm('))
      .map(rel)
    expect(callers).toEqual(['src/components/Toolbar.tsx'])
  })

  it('.dialog 有高度上限且自己能滚', () => {
    const rule = cssRule('.dialog')
    expect(rule).toContain('max-height: 90vh')
    expect(rule).toContain('overflow-y: auto')
  })

  it('.dialog-actions 贴在框底', () => {
    const rule = cssRule('.dialog-actions')
    expect(rule).toContain('position: sticky')
    expect(rule).toContain('bottom: 0')
  })
})
