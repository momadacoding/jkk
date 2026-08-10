#!/usr/bin/env node
/**
 * 生成 THIRD_PARTY_NOTICES.md —— 随安装包分发的第三方许可声明。
 *
 * 为什么必须有：MIT 要求「在所有副本或实质性部分中包含上述版权声明和许可声明」，
 * Apache-2.0 §4 要求向接收者提供许可副本并保留声明。我们分发的是把这些依赖
 * 编译进去的 .msi / .exe，接收者拿不到 node_modules 和 Cargo.toml，
 * 所以声明必须随包走，不能只留一句「自己去锁文件里查」。
 *
 * 只收**真正进入二进制**的依赖：
 *   - Rust：从 root package 出发，只跟 normal 依赖，按目标平台过滤；
 *           dev-dependencies（测试用）和 build-dependencies（只在构建期跑）不分发。
 *   - npm： `pnpm list --prod`，devDependencies（vite / vitest / typescript）不进产物。
 *
 * 用法：
 *   node scripts/gen-notices.mjs           写入 THIRD_PARTY_NOTICES.md
 *   node scripts/gen-notices.mjs --check   只比对，有漂移就非零退出（CI 用）
 */
import { execFileSync } from 'node:child_process'
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const outFile = path.join(repoRoot, 'THIRD_PARTY_NOTICES.md')
const checkOnly = process.argv.includes('--check')

/** 许可正文文件的常见名字。 */
const LICENSE_FILE = /^(LICENSE|LICENCE|COPYING|NOTICE)([-.].*)?$/i

/**
 * 不必进去找许可的子目录。剩下的一层子目录都要扫——`*-sys` 类 crate
 * 会把上游 C 代码整个 vendor 进一个子目录，而那份代码的许可往往和 crate
 * 自己声明的完全不同。
 *
 * 实例：`libgit2-sys` 根目录只有 LICENSE-APACHE / LICENSE-MIT（Rust binding 的），
 * 真正约束分发的 GPL-2.0 + 链接例外在 `libgit2/COPYING`。只扫根目录会把
 * 整棵树里最关键的一份许可漏掉。
 */
const SKIP_DIRS = new Set([
  'src', 'tests', 'test', 'examples', 'example', 'benches', 'bench',
  'docs', 'doc', 'ci', '.github', '.git', 'target', 'node_modules',
  'scripts', 'script', 'include', 'cmake', 'fuzz',
])

/** 读一个目录下的许可正文文件（不递归）。 */
function readDirLicenses(dir, prefix) {
  let entries
  try {
    entries = readdirSync(dir, { withFileTypes: true })
  } catch {
    return []
  }
  return entries
    .filter((e) => e.isFile() && LICENSE_FILE.test(e.name))
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((e) => {
      let text = ''
      try {
        text = readFileSync(path.join(dir, e.name), 'utf8').replace(/\r\n/g, '\n').trim()
      } catch {}
      return { name: prefix ? `${prefix}/${e.name}` : e.name, text }
    })
    .filter((f) => f.text.length > 0)
}

/** 在包目录及其一层子目录里找许可正文。 */
function readLicenseTexts(dir) {
  if (!dir || !existsSync(dir)) return []
  const out = readDirLicenses(dir, '')

  let subdirs = []
  try {
    subdirs = readdirSync(dir, { withFileTypes: true })
      .filter((e) => e.isDirectory() && !SKIP_DIRS.has(e.name.toLowerCase()) && !e.name.startsWith('.'))
      .map((e) => e.name)
      .sort()
  } catch {}

  for (const sub of subdirs) {
    out.push(...readDirLicenses(path.join(dir, sub), sub))
  }
  return out
}

// ---------------------------------------------------------------- Rust

function rustPackages() {
  const raw = execFileSync(
    'cargo',
    [
      'metadata',
      '--format-version',
      '1',
      '--manifest-path',
      path.join(repoRoot, 'src-tauri', 'Cargo.toml'),
      // 平台相关依赖（比如各平台的 winapi / objc）只保留本目标用得上的那些
      '--filter-platform',
      'x86_64-pc-windows-msvc',
    ],
    { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  )
  const meta = JSON.parse(raw)
  const byId = new Map(meta.packages.map((p) => [p.id, p]))
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]))
  const rootId = meta.resolve.root ?? meta.workspace_members[0]

  // 只跟 normal 依赖。dep_kinds 里 kind 为 null 表示 normal；
  // "dev" 只在测试时链接，"build" 只在构建期跑，两者都不进分发的二进制。
  const seen = new Set()
  const queue = [rootId]
  while (queue.length) {
    const id = queue.shift()
    if (seen.has(id)) continue
    seen.add(id)
    const node = nodes.get(id)
    if (!node) continue
    for (const dep of node.deps) {
      const isNormal = dep.dep_kinds.some((k) => k.kind === null || k.kind === undefined)
      if (isNormal) queue.push(dep.pkg)
    }
  }
  seen.delete(rootId)

  return [...seen]
    .map((id) => byId.get(id))
    .filter(Boolean)
    .map((p) => ({
      ecosystem: 'cargo',
      name: p.name,
      version: p.version,
      license: p.license ?? (p.license_file ? `see ${p.license_file}` : 'UNKNOWN'),
      repository: p.repository ?? null,
      // manifest_path 指向 Cargo.toml，许可正文在它旁边
      dir: p.manifest_path ? path.dirname(p.manifest_path) : null,
    }))
}

// ---------------------------------------------------------------- npm

function npmPackages() {
  const raw = execFileSync(
    'pnpm',
    ['list', '--prod', '--depth', 'Infinity', '--json'],
    { cwd: repoRoot, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, shell: process.platform === 'win32' },
  )
  const roots = JSON.parse(raw)
  const found = new Map()

  const walk = (deps) => {
    for (const [name, info] of Object.entries(deps ?? {})) {
      if (!info || typeof info !== 'object') continue
      const key = `${name}@${info.version}`
      if (!found.has(key)) {
        let license = 'UNKNOWN'
        let repository = null
        const dir = info.path ?? null
        if (dir && existsSync(path.join(dir, 'package.json'))) {
          try {
            const pj = JSON.parse(readFileSync(path.join(dir, 'package.json'), 'utf8'))
            license =
              typeof pj.license === 'string'
                ? pj.license
                : pj.license?.type ?? (Array.isArray(pj.licenses) ? pj.licenses.map((l) => l.type).join(' OR ') : 'UNKNOWN')
            repository = typeof pj.repository === 'string' ? pj.repository : pj.repository?.url ?? null
          } catch {}
        }
        found.set(key, { ecosystem: 'npm', name, version: info.version, license, repository, dir })
        walk(info.dependencies)
      }
    }
  }
  for (const r of roots) {
    walk(r.dependencies)
    walk(r.optionalDependencies)
  }
  return [...found.values()]
}

// ---------------------------------------------------------------- render

function render(packages) {
  const sorted = packages.sort(
    (a, b) => a.ecosystem.localeCompare(b.ecosystem) || a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
  )

  const lines = []
  lines.push('# Third-Party Notices')
  lines.push('')
  lines.push('本文件由 `scripts/gen-notices.mjs` 从锁文件生成，**请勿手工编辑**。')
  lines.push('重新生成：`node scripts/gen-notices.mjs`。')
  lines.push('')
  lines.push('jkk 分发的二进制包含下列第三方组件。每个组件均遵循其自身的许可条款，')
  lines.push('相关版权声明与许可正文一并附于本文件，以满足 MIT / Apache-2.0 等许可')
  lines.push('对分发物保留声明的要求。')
  lines.push('')
  lines.push('收录范围仅限**实际进入二进制**的依赖：Rust 侧为 normal 依赖（不含')
  lines.push('dev-dependencies 与 build-dependencies），npm 侧为 production 依赖')
  lines.push('（不含 devDependencies）。')
  lines.push('')

  // 汇总表
  lines.push('## 组件清单')
  lines.push('')
  lines.push(`共 ${sorted.length} 个组件。`)
  lines.push('')
  lines.push('| 来源 | 组件 | 版本 | 许可 |')
  lines.push('|---|---|---|---|')
  for (const p of sorted) {
    lines.push(`| ${p.ecosystem} | ${p.name} | ${p.version} | ${p.license} |`)
  }
  lines.push('')

  // 许可正文，按内容去重：同一份 MIT 正文被几百个包共用时没必要重复几百遍
  lines.push('## 许可正文')
  lines.push('')
  const texts = new Map() // text -> package labels
  const missing = []
  for (const p of sorted) {
    const files = readLicenseTexts(p.dir)
    if (files.length === 0) {
      missing.push(p)
      continue
    }
    for (const f of files) {
      const label = `${p.name} ${p.version} (${p.ecosystem})`
      if (!texts.has(f.text)) texts.set(f.text, new Set())
      texts.get(f.text).add(label)
    }
  }

  const blocks = [...texts.entries()]
    .map(([text, labels]) => ({ text, labels: [...labels].sort() }))
    .sort((a, b) => a.labels[0].localeCompare(b.labels[0]))

  for (const b of blocks) {
    lines.push('<details>')
    lines.push(`<summary>${b.labels.length} 个组件共用此许可正文：${b.labels[0]}${b.labels.length > 1 ? ' 等' : ''}</summary>`)
    lines.push('')
    lines.push('适用于：')
    for (const l of b.labels) lines.push(`- ${l}`)
    lines.push('')
    lines.push('```text')
    lines.push(b.text)
    lines.push('```')
    lines.push('')
    lines.push('</details>')
    lines.push('')
  }

  if (missing.length) {
    lines.push('## 未随包附带许可正文的组件')
    lines.push('')
    lines.push('下列组件在其分发包中未包含许可正文文件，只能依据其清单声明的 SPDX 标识；')
    lines.push('对应许可的标准正文见上，或见该组件的源码仓库。')
    lines.push('')
    for (const p of missing) {
      lines.push(`- ${p.name} ${p.version} (${p.ecosystem}) — ${p.license}${p.repository ? ` — ${p.repository}` : ''}`)
    }
    lines.push('')
  }

  return lines.join('\n') + '\n'
}

// ---------------------------------------------------------------- main

const packages = [...rustPackages(), ...npmPackages()]
const content = render(packages)

if (checkOnly) {
  if (!existsSync(outFile)) {
    console.error('THIRD_PARTY_NOTICES.md 不存在。运行 node scripts/gen-notices.mjs 生成。')
    process.exit(1)
  }
  const current = readFileSync(outFile, 'utf8').replace(/\r\n/g, '\n')
  if (current !== content) {
    console.error(
      'THIRD_PARTY_NOTICES.md 与当前依赖不一致——依赖动过但没重新生成。\n' +
        '运行 node scripts/gen-notices.mjs 并提交结果。',
    )
    process.exit(1)
  }
  console.log(`THIRD_PARTY_NOTICES.md 是最新的（${packages.length} 个组件）。`)
} else {
  writeFileSync(outFile, content)
  console.log(`wrote ${path.relative(repoRoot, outFile)} —— ${packages.length} 个组件`)
}
