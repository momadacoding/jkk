// 顶部不写 shebang：这个文件同时被 vitest 从 .ts 测试导入，CRLF 检出时 shebang 会让 Vite 解析报错（v0.8.2 发版预检就挂在这里）。
// 盘点各 worktree 的 src-tauri/target 占了多少盘，按需删掉用不着的（第 83 条方案 B）。只用 Node 内置模块。
//
//   node scripts/worktree-targets.mjs                 只读：列出每个 target 的大小与能不能清
//   node scripts/worktree-targets.mjs --clean         删掉标为「可清」的 target，删之前要输入 yes
//   node scripts/worktree-targets.mjs --clean --all   开着 PR、没有 PR 的 worktree 也一起清
//   node scripts/worktree-targets.mjs --clean --yes   不问（非交互环境必须带）
//
// 只删 target（`cargo build` 能重新生成），不删 worktree、不删分支、不碰 git。
// 为什么每个 worktree 各有一份 target、以及为什么暂不共享 CARGO_TARGET_DIR，见 README「清理 worktree 的编译产物」。

import { execFileSync } from 'node:child_process'
import { existsSync, realpathSync } from 'node:fs'
import { lstat, readdir, rm, stat } from 'node:fs/promises'
import path from 'node:path'
import { createInterface } from 'node:readline/promises'
import { fileURLToPath } from 'node:url'

/** target 里最新的文件在这么多分钟内写过，就当它可能正在编译，不清。 */
export const RECENT_MINUTES = 30

/**
 * 解析 `git worktree list --porcelain`。
 * @returns {{ path: string, branch: string | null, locked: string | null, prunable: boolean }[]}
 *   `locked` 是 lock 的原因（没写原因时为空串），没 lock 为 null；分离 HEAD 时 `branch` 为 null。
 */
export function parseWorktreeList(text) {
  const out = []
  let cur = null
  for (const line of text.split(/\r?\n/)) {
    if (line.startsWith('worktree ')) {
      cur = { path: line.slice('worktree '.length), branch: null, locked: null, prunable: false }
      out.push(cur)
    } else if (!cur) {
      continue
    } else if (line.startsWith('branch refs/heads/')) {
      cur.branch = line.slice('branch refs/heads/'.length)
    } else if (line === 'locked' || line.startsWith('locked ')) {
      cur.locked = line.slice('locked'.length).trim()
    } else if (line === 'prunable' || line.startsWith('prunable ')) {
      cur.prunable = true
    }
  }
  return out
}

/** 分支名 → 它最新的那条 PR（同名分支可能开过多条，取编号最大的）。 */
export function latestPrByBranch(prs) {
  const map = new Map()
  for (const pr of prs) {
    const prev = map.get(pr.headRefName)
    if (!prev || pr.number > prev.number) map.set(pr.headRefName, pr)
  }
  return map
}

/**
 * 一个 target 能不能清。
 *
 * 先过「任何时候都不清」的几道：主工作区、正在其中运行脚本的目录（当前目录或脚本本身所在）、
 * 被 lock 的（Claude Code 给活跃会话加的锁）、
 * 有进程从 target 里运行的、近期还在写入的（可能正在编译）。之后：残留目录与 PR 已合并 / 已关闭的清，
 * 开着 PR、没有 PR、读不到 PR 的只在 `--all` 时清。
 *
 * @param {{ kind: 'main' | 'current' | 'worktree' | 'orphan', locked: string | null,
 *   pr: { number: number, state: string } | null | undefined, busy: string[], newestMtimeMs: number | null }} entry
 *   `pr`：null = 没有 PR，undefined = 读不到 PR 列表
 * @param {{ all: boolean, nowMs: number }} opts
 * @returns {{ clean: boolean, reason: string }}
 */
export function classify(entry, { all, nowMs }) {
  if (entry.kind === 'main') return { clean: false, reason: '主工作区' }
  if (entry.kind === 'current') return { clean: false, reason: '正在这个目录里运行脚本' }
  if (entry.locked !== null && entry.locked !== undefined) {
    return { clean: false, reason: entry.locked ? `被 lock：${entry.locked}` : '被 lock' }
  }
  if (entry.busy.length > 0) return { clean: false, reason: `有进程正在用：${[...new Set(entry.busy)].join('、')}` }
  if (entry.newestMtimeMs !== null && nowMs - entry.newestMtimeMs <= RECENT_MINUTES * 60 * 1000) {
    return { clean: false, reason: `target ${RECENT_MINUTES} 分钟内还有写入，可能正在编译` }
  }
  if (entry.kind === 'orphan') return { clean: true, reason: '没登记在 git 里的残留目录' }

  const { pr } = entry
  if (pr && pr.state === 'MERGED') return { clean: true, reason: `PR 第 ${pr.number} 条已合并` }
  if (pr && pr.state === 'CLOSED') return { clean: true, reason: `PR 第 ${pr.number} 条已关闭` }
  const what = pr ? `PR 第 ${pr.number} 条还开着` : pr === null ? '没有 PR' : '读不到 PR 状态'
  return all ? { clean: true, reason: `${what}（--all）` } : { clean: false, reason: `${what}（加 --all 一起清）` }
}

export function formatBytes(n) {
  if (n < 1024) return `${n} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v.toFixed(1)} ${units[i]}`
}

/**
 * 比较用的路径：先解析成真实路径（junction、符号链接、8.3 短名、subst 盘符都归一），
 * Windows 上再折叠大小写、统一反斜杠（git 给正斜杠，进程路径是反斜杠）。路径不存在时退回字面解析。
 */
function pathKey(p) {
  let abs
  try {
    abs = realpathSync.native(p)
  } catch {
    abs = path.resolve(p)
  }
  return process.platform === 'win32' ? abs.toLowerCase() : abs
}

/** child 是否等于 dir 或在 dir 里面。两边都要先过 `pathKey`。 */
function isWithinKey(childKey, dirKey) {
  const rel = path.relative(dirKey, childKey)
  return rel === '' || (!rel.startsWith('..') && !path.isAbsolute(rel))
}

/** 进程的可执行文件是不是在 target 里。 */
function runsFrom(exe, target) {
  return Boolean(exe) && isWithinKey(pathKey(exe), pathKey(target))
}

/**
 * 目录总大小、文件数、最新文件的修改时间。不跟随符号链接 / junction：只算链接本身。
 * 硬链接只算一次：cargo 会把 deps/ 里的产物硬链接到 debug/，按条目累加会把同一份数据算两遍。
 */
async function measure(dir) {
  let bytes = 0
  let files = 0
  let newestMtimeMs = null
  const seen = new Set()
  const stack = [dir]
  while (stack.length > 0) {
    const cur = stack.pop()
    let entries
    try {
      entries = await readdir(cur, { withFileTypes: true })
    } catch {
      continue // 边量边被删、没权限：少算一点不影响结论
    }
    await Promise.all(
      entries.map(async (e) => {
        const full = path.join(cur, e.name)
        if (e.isDirectory()) {
          stack.push(full)
          return
        }
        try {
          // bigint：Windows 上的文件编号超过 2^53，转成 number 会让不同文件撞号
          const st = await lstat(full, { bigint: true })
          if (st.nlink > 1n && st.ino !== 0n) {
            const id = `${st.dev}:${st.ino}`
            if (seen.has(id)) return
            seen.add(id)
          }
          bytes += Number(st.size)
          files++
          const mtime = Number(st.mtimeMs)
          if (newestMtimeMs === null || mtime > newestMtimeMs) newestMtimeMs = mtime
        } catch {
          // 同上
        }
      }),
    )
  }
  return { bytes, files, newestMtimeMs }
}

/** target 本身和往下两层目录里，有没有在 `RECENT_MINUTES` 内改过的：cargo 往 deps/、.fingerprint/ 里写文件会更新这些目录的时间戳。 */
async function touchedRecently(target, nowMs) {
  const limit = RECENT_MINUTES * 60 * 1000
  let level = [target]
  for (let depth = 0; depth <= 2 && level.length > 0; depth++) {
    const next = []
    for (const dir of level) {
      try {
        if (nowMs - (await stat(dir)).mtimeMs <= limit) return true
        if (depth < 2) {
          for (const e of await readdir(dir, { withFileTypes: true })) {
            if (e.isDirectory()) next.push(path.join(dir, e.name))
          }
        }
      } catch {
        // 正被删、没权限：按没写过算，后面 rm 失败会如实报告
      }
    }
    level = next
  }
  return false
}

function git(cwd, args) {
  return execFileSync('git', ['-C', cwd, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })
}

function readWorktrees(repo) {
  const worktrees = parseWorktreeList(git(repo, ['worktree', 'list', '--porcelain']))
  if (worktrees.length === 0) throw new Error(`${repo} 里读不到 worktree 列表`)
  return worktrees
}

/**
 * 收集每个有 target 的 worktree（含主工作区与没登记在 git 里的残留目录），量大小并给出结论。
 *
 * @param {{ repo: string, cwd: string, scriptDir?: string, prs: object[] | undefined, processPaths: string[],
 *   nowMs: number, all: boolean }} opts
 *   `prs` 为 undefined 表示读不到 PR 列表；`processPaths` 是正在运行的进程的可执行文件路径；
 *   `cwd` 与 `scriptDir` 所在的那个目录（取最深的一个）标为 current，任何时候都不清
 */
export async function collect({ repo, cwd, scriptDir, prs, processPaths, nowMs, all }) {
  const worktrees = readWorktrees(repo)
  const mainPath = worktrees[0].path
  const prMap = prs ? latestPrByBranch(prs) : null

  // prunable = 目录没了或其中的 .git 文件没了。目录还在的话按残留目录处理，所以不算「已登记」
  const live = worktrees.filter((w) => !w.prunable)
  const candidates = live.map((w) => ({
    path: w.path,
    branch: w.branch,
    locked: w.locked,
    kind: w === worktrees[0] ? 'main' : 'worktree',
  }))
  // worktree remove 在 Windows 上常删不干净，留下没登记的目录
  const registered = new Set(live.map((w) => pathKey(w.path)))
  for (const parent of ['.claude/worktrees', '.worktrees']) {
    let entries = []
    try {
      entries = await readdir(path.join(mainPath, parent), { withFileTypes: true })
    } catch {
      continue
    }
    for (const e of entries) {
      const full = path.join(mainPath, parent, e.name)
      if (e.isDirectory() && !registered.has(pathKey(full))) {
        candidates.push({ path: full, branch: null, locked: null, kind: 'orphan' })
      }
    }
  }

  // worktree 都在主工作区下面，所以「站在哪个目录里」取路径最深的那个候选
  for (const here of [cwd, scriptDir].filter(Boolean)) {
    const key = pathKey(here)
    const hit = candidates
      .filter((c) => c.kind !== 'main' && isWithinKey(key, pathKey(c.path)))
      .sort((a, b) => pathKey(b.path).length - pathKey(a.path).length)[0]
    if (hit) hit.kind = 'current'
  }

  const rows = await Promise.all(
    candidates.map(async (c) => {
      const target = path.join(c.path, 'src-tauri', 'target')
      let link
      try {
        link = (await lstat(target)).isSymbolicLink()
      } catch {
        return null // 没有 target
      }
      // git 给正斜杠、目录扫描给反斜杠：显示前统一成本平台写法
      const shown = path.resolve(c.path)
      const base = { name: path.basename(shown), path: shown, kind: c.kind, branch: c.branch, target }
      // 共享 target（第 83 条方案 A）的形态：顺着量会把同一份算进好几个 worktree，删也删的是别人的
      if (link) return { ...base, bytes: 0, files: 0, newestMtimeMs: null, clean: false, reason: 'target 是链接，不处理' }
      const { bytes, files, newestMtimeMs } = await measure(target)
      const busy = processPaths.filter((p) => runsFrom(p, target)).map((p) => path.basename(p))
      const pr = c.kind === 'orphan' ? null : prMap ? (c.branch && prMap.get(c.branch)) || null : undefined
      const verdict = classify({ kind: c.kind, locked: c.locked, pr, busy, newestMtimeMs }, { all, nowMs })
      return { ...base, bytes, files, newestMtimeMs, ...verdict }
    }),
  )
  return rows.filter(Boolean).sort((a, b) => b.bytes - a.bytes)
}

/**
 * 删除前一刻的复查（独立 review 第 1 条）：盘点要量几十 GB、确认又要等人，这段时间里别的会话可能已经进了
 * 某个 worktree（加了 lock）、开始编译或跑起了 app.exe。每删一个之前重新看一遍 lock、进程、近期写入。
 *
 * @param {{ repo: string, readProcessPaths: () => string[], nowMs: () => number }} deps
 * @returns {(row: object) => Promise<string | null>} 返回跳过的原因；null = 可以删
 */
export function makeRecheck({ repo, readProcessPaths, nowMs }) {
  return async (row) => {
    const key = pathKey(row.path)
    const entry = readWorktrees(repo).find((w) => !w.prunable && pathKey(w.path) === key)
    if (entry && entry.locked !== null) {
      return entry.locked ? `盘点之后被 lock 了：${entry.locked}` : '盘点之后被 lock 了'
    }
    if (row.kind === 'orphan' && entry) return '盘点之后登记成了 worktree'
    const busy = readProcessPaths().filter((p) => runsFrom(p, row.target)).map((p) => path.basename(p))
    if (busy.length > 0) return `盘点之后有进程在用：${[...new Set(busy)].join('、')}`
    if (await touchedRecently(row.target, nowMs())) return `盘点之后 ${RECENT_MINUTES} 分钟内有写入，可能正在编译`
    return null
  }
}

/**
 * 逐个删除 target。一个失败不影响其余；删完再看一眼目录还在不在，以实际结果为准。
 * @param {(row: object) => Promise<string | null>} [recheck] 删每一个之前调一次，返回原因就跳过（`makeRecheck`）
 * @returns {Promise<{ target: string, ok: boolean, skipped?: string, error?: string }[]>}
 */
export async function cleanTargets(rows, recheck) {
  const results = []
  for (const row of rows) {
    const skipped = recheck ? await recheck(row) : null
    if (skipped) {
      results.push({ target: row.target, ok: false, skipped })
      continue
    }
    // 兜底：只认 `<某目录>/src-tauri/target`，传错了行也删不到别处
    if (path.basename(row.target) !== 'target' || path.basename(path.dirname(row.target)) !== 'src-tauri') {
      results.push({ target: row.target, ok: false, error: '不是 src-tauri/target，拒绝删除' })
      continue
    }
    let error
    try {
      await rm(row.target, { recursive: true, force: true, maxRetries: 3, retryDelay: 200 })
    } catch (e) {
      error = e instanceof Error ? e.message : String(e)
    }
    const ok = !existsSync(row.target)
    results.push(ok ? { target: row.target, ok } : { target: row.target, ok, error: error ?? '删完目录还在' })
  }
  return results
}

/** 正在运行的进程的可执行文件路径。只在 Windows 上查（主力平台）；读不到返回空，结论里会少一道「占用」检查。 */
function runningProcessPaths() {
  if (process.platform !== 'win32') return []
  try {
    const out = execFileSync(
      'powershell.exe',
      ['-NoProfile', '-NonInteractive', '-Command', '[Console]::OutputEncoding = [Text.Encoding]::UTF8; (Get-Process).Path'],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] },
    )
    return out.split(/\r?\n/).filter(Boolean)
  } catch {
    console.warn('注意：读不到进程列表，没法检查 target 有没有进程在用')
    return []
  }
}

function readPrs(repo) {
  try {
    const out = execFileSync('gh', ['pr', 'list', '--state', 'all', '--limit', '1000', '--json', 'number,headRefName,state'], {
      cwd: repo,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    return JSON.parse(out)
  } catch {
    console.warn('注意：读不到 PR 列表（gh 没装、没登录或没网络），只有残留目录会标为可清；加 --all 一起清')
    return undefined
  }
}

function usage() {
  console.error('用法：node scripts/worktree-targets.mjs [--clean [--all] [--yes]] [--repo <路径>]')
  return 2
}

async function main(argv) {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url))
  const opts = { clean: false, all: false, yes: false, repo: path.resolve(scriptDir, '..') }
  const args = argv.slice(2)
  for (let i = 0; i < args.length; i++) {
    const a = args[i]
    if (a === '--clean') opts.clean = true
    else if (a === '--all') opts.all = true
    else if (a === '--yes') opts.yes = true
    else if (a === '--repo' && args[i + 1]) opts.repo = args[++i]
    else return usage()
  }
  if ((opts.all || opts.yes) && !opts.clean) return usage()

  const rows = await collect({
    repo: opts.repo,
    cwd: process.cwd(),
    scriptDir,
    prs: readPrs(opts.repo),
    processPaths: runningProcessPaths(),
    nowMs: Date.now(),
    all: opts.all,
  })
  if (rows.length === 0) {
    console.log('没有找到 src-tauri/target')
    return 0
  }
  for (const r of rows) {
    console.log(`${r.clean ? '可清' : '保留'}  ${formatBytes(r.bytes).padStart(9)}  ${r.path}`)
    console.log(`                 ${r.reason}`)
  }
  const picked = rows.filter((r) => r.clean)
  const sum = (list) => formatBytes(list.reduce((n, r) => n + r.bytes, 0))
  console.log(`\n合计 ${sum(rows)}，其中可清 ${picked.length} 个、${sum(picked)}`)
  if (!opts.clean) {
    if (picked.length > 0) console.log('加 --clean 删除标为「可清」的 target（会先确认）')
    return 0
  }
  if (picked.length === 0) return 0

  if (!opts.yes) {
    if (!process.stdin.isTTY) {
      console.error('不是交互终端，没法确认；确定要删就加 --yes')
      return 2
    }
    const rl = createInterface({ input: process.stdin, output: process.stdout })
    const answer = await rl.question(`删除以上 ${picked.length} 个 target（${sum(picked)}）？输入 yes 确认：`)
    rl.close()
    if (answer.trim() !== 'yes') {
      console.log('没有删除任何东西')
      return 0
    }
  }
  const recheck = makeRecheck({ repo: opts.repo, readProcessPaths: runningProcessPaths, nowMs: Date.now })
  const results = await cleanTargets(picked, recheck)
  for (const r of results) {
    if (r.skipped) console.log(`跳过    ${r.target}：${r.skipped}`)
    else console.log(`${r.ok ? '已删除' : '失败  '}  ${r.target}${r.error ? `：${r.error}` : ''}`)
  }
  const failed = results.filter((r) => !r.ok && !r.skipped)
  if (failed.length > 0) {
    console.error(`\n${failed.length} 个没删干净：多半是有文件正被占用。剩下的部分重新编译时会被覆盖，也可以关掉占用的程序后重跑`)
    return 1
  }
  return 0
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
if (isMain) {
  main(process.argv).then(
    (code) => {
      process.exitCode = code
    },
    (err) => {
      console.error(err instanceof Error ? err.message : err)
      process.exitCode = 1
    },
  )
}
