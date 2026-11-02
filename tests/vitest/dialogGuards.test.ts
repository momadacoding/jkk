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
