/**
 * 被 vitest 导入的 `scripts/` 脚本不能以 shebang 开头。
 *
 * Vite 导入带 `#!` 的文件，在 CRLF 检出（CI 的 `windows-latest`、主工作区，都按 `core.autocrlf`）时报
 * `SyntaxError: Invalid or unexpected token`；工具在 worktree 里直接写出的文件是 LF，本地测试照样通过。
 * v0.8.2 的发版预检就这样挂在 `perf-summary.mjs` 上。这里读源码判断，不依赖本机的换行方式。
 */
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const ROOT = process.cwd()
const TESTS = join(ROOT, 'tests', 'vitest')
const SCRIPTS = join(ROOT, 'scripts')

/** 测试文件里指向 `scripts/` 的相对导入，解析成绝对路径。 */
function scriptImports(testFile: string, source: string): string[] {
  // 三种写法：`... from '<spec>'`、`import('<spec>')`、纯副作用的 `import '<spec>'`
  const specs = [
    ...source.matchAll(
      /\bfrom\s+['"](\.{1,2}\/[^'"]+)['"]|\bimport\s*\(\s*['"](\.{1,2}\/[^'"]+)['"]\s*\)|\bimport\s+['"](\.{1,2}\/[^'"]+)['"]/g,
    ),
  ].map((m) => m[1] ?? m[2] ?? m[3])
  return specs
    .map((spec) => resolve(dirname(testFile), spec))
    .filter((target) => !relative(SCRIPTS, target).startsWith('..'))
}

function testFiles(): string[] {
  return readdirSync(TESTS, { recursive: true, encoding: 'utf8' })
    // 不只测试文件：辅助模块导入的脚本同样会被 Vite 加载
    .filter((name) => /\.(tsx?|m?js)$/.test(name))
    .map((name) => join(TESTS, name))
    // 本文件的用例里写着示例导入，不是真的导入
    .filter((file) => resolve(file) !== resolve(TESTS, 'scriptImportGuards.test.ts'))
}

describe('scriptImports', () => {
  it('只挑出指向 scripts/ 的相对导入，静态和动态 import 都算', () => {
    const file = join(TESTS, 'x.test.ts')
    const source = [
      "import { a } from '../../scripts/a.mjs'",
      "import { b } from '../../src/b'",
      "import { c } from 'vitest'",
      "const d = await import('../../scripts/d.mjs')",
      "import '../../scripts/e.mjs'",
      "import { f } from '../../scripts-other/f.mjs'",
    ].join('\n')
    expect(scriptImports(file, source)).toEqual([join(SCRIPTS, 'a.mjs'), join(SCRIPTS, 'd.mjs'), join(SCRIPTS, 'e.mjs')])
  })
})

describe('被测试导入的脚本', () => {
  const targets = [...new Set(testFiles().flatMap((f) => scriptImports(f, readFileSync(f, 'utf8'))))]

  it('能找到现有的导入（防止规则写错后空转）', () => {
    expect(targets).toContain(join(SCRIPTS, 'gen-notices.mjs'))
    expect(targets).toContain(join(SCRIPTS, 'perf-summary.mjs'))
  })

  it('都能直接对上文件（不带扩展名的导入会让下面的检查静默跳过）', () => {
    expect(targets.filter((t) => !existsSync(t)).map((t) => relative(ROOT, t))).toEqual([])
  })

  it('开头都没有 shebang', () => {
    const offenders = targets.filter((t) => existsSync(t) && readFileSync(t, 'utf8').startsWith('#!'))
    expect(offenders.map((t) => relative(ROOT, t))).toEqual([])
  })
})
