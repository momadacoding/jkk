import { execFileSync } from 'node:child_process'
import {
  existsSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  symlinkSync,
  utimesSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import {
  RECENT_MINUTES,
  classify,
  cleanTargets,
  collect,
  formatBytes,
  latestPrByBranch,
  makeRecheck,
  parseWorktreeList,
} from '../../scripts/worktree-targets.mjs'

describe('parseWorktreeList', () => {
  // 形状照抄 2026-11-17 本仓库 `git worktree list --porcelain` 的真实输出
  const porcelain = [
    'worktree C:/repo',
    'HEAD 6b0dc52ad28d5a017c23f97a4b486646ed3f3d9c',
    'branch refs/heads/master',
    '',
    'worktree C:/repo/.claude/worktrees/a',
    'HEAD 7fd4c0767958464e29eed811d099a7667e2eabee',
    'branch refs/heads/worktree-a',
    '',
    'worktree C:/repo/.claude/worktrees/b',
    'HEAD 6b0dc52ad28d5a017c23f97a4b486646ed3f3d9c',
    'branch refs/heads/worktree-b',
    'locked claude session b (pid 36508)',
    '',
    'worktree C:/repo/.worktrees/detached',
    'HEAD 6b0dc52ad28d5a017c23f97a4b486646ed3f3d9c',
    'detached',
    '',
    'worktree C:/repo/.claude/worktrees/gone',
    'HEAD 6b0dc52ad28d5a017c23f97a4b486646ed3f3d9c',
    'branch refs/heads/worktree-gone',
    'prunable gitdir file points to non-existent location',
    '',
  ].join('\n')

  it('逐条取出路径、分支、lock 原因、prunable', () => {
    expect(parseWorktreeList(porcelain)).toEqual([
      { path: 'C:/repo', branch: 'master', locked: null, prunable: false },
      { path: 'C:/repo/.claude/worktrees/a', branch: 'worktree-a', locked: null, prunable: false },
      { path: 'C:/repo/.claude/worktrees/b', branch: 'worktree-b', locked: 'claude session b (pid 36508)', prunable: false },
      { path: 'C:/repo/.worktrees/detached', branch: null, locked: null, prunable: false },
      { path: 'C:/repo/.claude/worktrees/gone', branch: 'worktree-gone', locked: null, prunable: true },
    ])
  })

  it('不带原因的 lock 也算 lock', () => {
    const [wt] = parseWorktreeList('worktree C:/x\nHEAD 1\nbranch refs/heads/x\nlocked\n')
    expect(wt.locked).toBe('')
  })

  it('CRLF 换行也认', () => {
    expect(parseWorktreeList('worktree C:/x\r\nHEAD 1\r\nbranch refs/heads/x\r\n')).toEqual([
      { path: 'C:/x', branch: 'x', locked: null, prunable: false },
    ])
  })

  it('路径带空格和中文原样保留', () => {
    expect(parseWorktreeList('worktree C:/代码/my repo\nHEAD 1\nbranch refs/heads/x\n')[0].path).toBe('C:/代码/my repo')
  })

  it('空输入得到空列表', () => {
    expect(parseWorktreeList('')).toEqual([])
  })
})

describe('latestPrByBranch', () => {
  it('同名分支开过多条 PR 时取编号最大的', () => {
    const map = latestPrByBranch([
      { number: 12, headRefName: 'x', state: 'MERGED' },
      { number: 40, headRefName: 'x', state: 'OPEN' },
      { number: 20, headRefName: 'x', state: 'CLOSED' },
      { number: 5, headRefName: 'y', state: 'MERGED' },
    ])
    expect(map.get('x')).toEqual({ number: 40, headRefName: 'x', state: 'OPEN' })
    expect(map.get('y')?.number).toBe(5)
  })
})

describe('classify', () => {
  const NOW = Date.UTC(2026, 10, 17, 12, 0, 0)
  const old = NOW - 24 * 60 * 60 * 1000
  const base = {
    kind: 'worktree' as const,
    branch: 'worktree-a',
    locked: null,
    pr: { number: 7, state: 'MERGED' },
    busy: [] as string[],
    newestMtimeMs: old,
  }
  const run = (over: Partial<typeof base> & Record<string, unknown> = {}, all = false) =>
    classify({ ...base, ...over }, { all, nowMs: NOW })

  it('PR 已合并：清', () => {
    expect(run()).toEqual({ clean: true, reason: 'PR 第 7 条已合并' })
  })

  it('PR 关闭没合并：清（分支不会再编译）', () => {
    expect(run({ pr: { number: 8, state: 'CLOSED' } })).toEqual({ clean: true, reason: 'PR 第 8 条已关闭' })
  })

  it('PR 还开着：默认不清，--all 才清', () => {
    expect(run({ pr: { number: 9, state: 'OPEN' } })).toEqual({ clean: false, reason: 'PR 第 9 条还开着（加 --all 一起清）' })
    expect(run({ pr: { number: 9, state: 'OPEN' } }, true)).toEqual({ clean: true, reason: 'PR 第 9 条还开着（--all）' })
  })

  it('没有 PR：默认不清，--all 才清', () => {
    expect(run({ pr: null })).toEqual({ clean: false, reason: '没有 PR（加 --all 一起清）' })
    expect(run({ pr: null }, true).clean).toBe(true)
  })

  it('读不到 PR（gh 不可用）：默认不清，--all 才清', () => {
    expect(run({ pr: undefined })).toEqual({ clean: false, reason: '读不到 PR 状态（加 --all 一起清）' })
    expect(run({ pr: undefined }, true).clean).toBe(true)
  })

  it('不归 git 管的残留目录：清', () => {
    expect(run({ kind: 'orphan', branch: null, pr: null })).toEqual({ clean: true, reason: '没登记在 git 里的残留目录' })
  })

  it('主工作区、当前所在的 worktree：任何时候都不清', () => {
    expect(run({ kind: 'main' }, true)).toEqual({ clean: false, reason: '主工作区' })
    expect(run({ kind: 'current' }, true)).toEqual({ clean: false, reason: '正在这个目录里运行脚本' })
  })

  it('被 lock：任何时候都不清，并带上原因', () => {
    expect(run({ locked: 'claude session b (pid 36508)' }, true)).toEqual({
      clean: false,
      reason: '被 lock：claude session b (pid 36508)',
    })
    expect(run({ locked: '' }, true)).toEqual({ clean: false, reason: '被 lock' })
  })

  it('有进程从 target 里运行：不清', () => {
    expect(run({ busy: ['app.exe', 'app.exe', 'bench.exe'] }, true)).toEqual({
      clean: false,
      reason: '有进程正在用：app.exe、bench.exe',
    })
  })

  it(`target 在 ${RECENT_MINUTES} 分钟内还有写入：不清（可能正在编译）`, () => {
    expect(run({ newestMtimeMs: NOW - 5 * 60 * 1000 }, true)).toEqual({
      clean: false,
      reason: `target ${RECENT_MINUTES} 分钟内还有写入，可能正在编译`,
    })
    expect(run({ newestMtimeMs: NOW - RECENT_MINUTES * 60 * 1000 - 1 }).clean).toBe(true)
  })

  it('残留目录同样要过占用和近期写入两道', () => {
    expect(run({ kind: 'orphan', pr: null, busy: ['app.exe'] }).clean).toBe(false)
    expect(run({ kind: 'orphan', pr: null, newestMtimeMs: NOW }).clean).toBe(false)
  })

  it('空 target（没有文件，newestMtimeMs 为 null）不算近期写入', () => {
    expect(run({ newestMtimeMs: null }).clean).toBe(true)
  })
})

describe('formatBytes', () => {
  it.each([
    [0, '0 B'],
    [1023, '1023 B'],
    [1536, '1.5 KB'],
    [5 * 1024 ** 2, '5.0 MB'],
    [12.34 * 1024 ** 3, '12.3 GB'],
  ])('%d → %s', (n, s) => {
    expect(formatBytes(n)).toBe(s)
  })
})

// 真实 git 仓库 + 真实 `git worktree list`：只有 PR 列表、进程列表、当前时间是注入的
describe('collect / cleanTargets（临时 fixture 仓库）', () => {
  let root: string
  let repo: string
  const NOW = Date.now()
  const OLD = new Date(NOW - 24 * 60 * 60 * 1000)
  const git = (cwd: string, ...args: string[]) => execFileSync('git', args, { cwd, stdio: 'pipe' })

  /**
   * 在 dir 下造一个 src-tauri/target，里面一个 `bytes` 字节的文件。文件和沿途目录的时间戳都拨到 `mtime`
   * （默认一天前）：删除前的复查看目录时间戳，刚建出来的目录会被当成「正在编译」。
   */
  function fakeTarget(dir: string, bytes: number, mtime = OLD) {
    const target = path.join(dir, 'src-tauri', 'target')
    const file = path.join(target, 'debug', 'deps', 'x.rlib')
    mkdirSync(path.dirname(file), { recursive: true })
    writeFileSync(file, Buffer.alloc(bytes))
    for (const p of [file, path.dirname(file), path.join(target, 'debug'), target]) utimesSync(p, mtime, mtime)
  }
  const wt = (name: string) => path.join(repo, '.claude', 'worktrees', name)
  const targetOf = (dir: string) => path.join(dir, 'src-tauri', 'target')

  beforeEach(() => {
    // CI runner 的 %TEMP% 是 8.3 短路径（C:\Users\RUNNER~1\...）。git 和脚本拿到的是长路径，测试自己拼的
    // 是短路径，断言里按字符串比 target 就对不上；不存在的 app.exe 也解析不成长路径。先在这里归一成长路径
    root = realpathSync.native(mkdtempSync(path.join(tmpdir(), 'wt-targets-')))
    repo = path.join(root, 'repo 带空格')
    mkdirSync(repo)
    git(repo, 'init', '-q', '-b', 'master')
    git(repo, 'config', 'user.email', 't@t')
    git(repo, 'config', 'user.name', 't')
    writeFileSync(path.join(repo, 'README.md'), 'x\n')
    git(repo, 'add', '.')
    git(repo, 'commit', '-qm', 'init')
    for (const name of ['merged', 'open', 'locked', 'busy', 'nopr', 'recent']) {
      git(repo, 'worktree', 'add', '-q', '-b', `worktree-${name}`, wt(name))
    }
    git(repo, 'worktree', 'lock', '--reason', 'claude session locked (pid 1)', wt('locked'))
    fakeTarget(repo, 10)
    fakeTarget(wt('merged'), 1000)
    fakeTarget(wt('open'), 2000)
    fakeTarget(wt('locked'), 3000)
    fakeTarget(wt('busy'), 4000)
    // 正在运行的 exe 删不掉、一定存在：pathKey 对它走 realpath，而不是解析失败后的字面退路（PR 186 review）
    writeFileSync(path.join(targetOf(wt('busy')), 'debug', 'app.exe'), '')
    utimesSync(path.join(targetOf(wt('busy')), 'debug'), OLD, OLD)
    fakeTarget(wt('nopr'), 5000)
    fakeTarget(wt('recent'), 6000, new Date(NOW - 60 * 1000))
    // worktree remove 删到一半留下的目录：git 已经不认它
    fakeTarget(wt('orphan'), 7000)
  })

  afterEach(() => {
    rmSync(root, { recursive: true, force: true })
  })

  const prs = [
    { number: 1, headRefName: 'worktree-merged', state: 'MERGED' },
    { number: 2, headRefName: 'worktree-open', state: 'OPEN' },
    { number: 3, headRefName: 'worktree-locked', state: 'MERGED' },
    { number: 4, headRefName: 'worktree-busy', state: 'MERGED' },
    { number: 6, headRefName: 'worktree-recent', state: 'MERGED' },
  ]
  /**
   * 进程路径故意写成和 worktree 路径**不同的形态**：正斜杠，Windows 上再整串大写。
   * 两边都走 path.join 的话分隔符天然一致，测不出比较逻辑（独立 review 第 8 条）。
   */
  const busyExe = () => {
    const p = path.join(targetOf(wt('busy')), 'debug', 'app.exe').replaceAll('\\', '/')
    return process.platform === 'win32' ? p.toUpperCase() : p
  }

  interface Row { name: string; kind: string; bytes: number; clean: boolean; reason: string; target: string }

  async function rowsOf(opts: { all?: boolean; cwd?: string } = {}): Promise<Row[]> {
    return collect({ repo, cwd: opts.cwd ?? repo, prs, processPaths: [busyExe()], nowMs: NOW, all: opts.all ?? false })
  }
  async function plan(opts: { all?: boolean; cwd?: string } = {}): Promise<Map<string, Row>> {
    return new Map((await rowsOf(opts)).map((r) => [r.name, r]))
  }
  const recheck = () => makeRecheck({ repo, readProcessPaths: () => [busyExe()], nowMs: () => NOW })

  it('每个有 target 的目录一行，带大小和结论', async () => {
    const rows = await plan()
    expect([...rows.keys()].sort()).toEqual(['busy', 'locked', 'merged', 'nopr', 'open', 'orphan', 'recent', 'repo 带空格'])
    expect(rows.get('merged')).toMatchObject({ kind: 'worktree', bytes: 1000, clean: true, reason: 'PR 第 1 条已合并' })
    expect(rows.get('repo 带空格')).toMatchObject({ kind: 'main', bytes: 10, clean: false })
    expect(rows.get('open')).toMatchObject({ clean: false })
    expect(rows.get('locked')).toMatchObject({ clean: false, reason: '被 lock：claude session locked (pid 1)' })
    // 进程名照进程列表原样显示（Windows 上这里被整串大写了）
    expect(rows.get('busy')).toMatchObject({ clean: false, reason: expect.stringMatching(/^有进程正在用：app\.exe$/i) })
    expect(rows.get('nopr')).toMatchObject({ clean: false })
    expect(rows.get('recent')).toMatchObject({ clean: false })
    expect(rows.get('orphan')).toMatchObject({ kind: 'orphan', bytes: 7000, clean: true })
  })

  it('--all 多清开着 PR 和没有 PR 的，lock / 占用 / 近期写入 / 主工作区仍不清', async () => {
    const rows = await plan({ all: true })
    const cleaned = [...rows.values()].filter((r) => r.clean).map((r) => r.name).sort()
    expect(cleaned).toEqual(['merged', 'nopr', 'open', 'orphan'])
  })

  it('站在某个 worktree 的子目录里运行：那个 worktree 不清', async () => {
    const rows = await plan({ cwd: path.join(wt('merged'), 'src-tauri'), all: true })
    expect(rows.get('merged')).toMatchObject({ kind: 'current', clean: false })
  })

  it('站在残留目录里运行：残留目录不清（不能因为 git 往上找到主工作区就漏掉）', async () => {
    const rows = await plan({ cwd: path.join(wt('orphan'), 'src-tauri') })
    expect(rows.get('orphan')).toMatchObject({ kind: 'current', clean: false })
  })

  it('经过别名路径登记的 worktree 不会被误认成残留目录', async () => {
    // 登记用的是 junction / 符号链接后的路径，扫描 .claude/worktrees 看到的是真实路径
    const alias = path.join(root, 'alias')
    symlinkSync(repo, alias, process.platform === 'win32' ? 'junction' : 'dir')
    const aliased = path.join(alias, '.claude', 'worktrees', 'aliased')
    git(repo, 'worktree', 'add', '-q', '-b', 'worktree-aliased', aliased)
    git(repo, 'worktree', 'lock', '--reason', 'in use', aliased)
    fakeTarget(aliased, 100)
    const rows = (await rowsOf({ all: true })).filter((r) => r.name === 'aliased')
    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({ kind: 'worktree', clean: false, reason: '被 lock：in use' })
  })

  it('目录还在、git 已经标成 prunable 的 worktree 按残留目录列出', async () => {
    // remove 删到一半：.git 文件没了，签出的文件和 target 还在
    rmSync(path.join(wt('nopr'), '.git'))
    const rows = await plan()
    expect(rows.get('nopr')).toMatchObject({ kind: 'orphan', bytes: 5000, clean: true })
  })

  it('硬链接只算一次（cargo 把 deps 里的产物硬链接到 debug/）', async () => {
    const deps = path.join(targetOf(wt('merged')), 'debug', 'deps')
    linkSync(path.join(deps, 'x.rlib'), path.join(targetOf(wt('merged')), 'debug', 'x.rlib'))
    const rows = await plan()
    expect(rows.get('merged')).toMatchObject({ bytes: 1000 })
  })

  it('target 本身是链接时不顺着量、也不清', async () => {
    // issue 第 83 条的方案 A（共享 target）就会是这个形态
    const shared = path.join(root, 'shared')
    fakeTarget(shared, 9000)
    rmSync(targetOf(wt('merged')), { recursive: true })
    symlinkSync(targetOf(shared), targetOf(wt('merged')), process.platform === 'win32' ? 'junction' : 'dir')
    const rows = await plan({ all: true })
    expect(rows.get('merged')).toMatchObject({ bytes: 0, clean: false, reason: 'target 是链接，不处理' })
  })

  it('删除前一刻复查：盘点之后才被 lock 的不删', async () => {
    const rows = (await rowsOf()).filter((r) => r.clean)
    git(repo, 'worktree', 'lock', '--reason', 'claude session merged (pid 2)', wt('merged'))
    const results: { target: string; ok: boolean; skipped?: string }[] = await cleanTargets(rows, recheck())
    const merged = results.find((r) => r.target === targetOf(wt('merged')))
    expect(merged).toMatchObject({ ok: false, skipped: '盘点之后被 lock 了：claude session merged (pid 2)' })
    expect(existsSync(targetOf(wt('merged')))).toBe(true)
    expect(existsSync(targetOf(wt('orphan')))).toBe(false)
  })

  it('删除前一刻复查：盘点之后才开始写入的不删', async () => {
    const rows = (await rowsOf()).filter((r) => r.clean)
    // 模拟开始编译：deps 下新建文件，目录时间戳随之更新
    writeFileSync(path.join(targetOf(wt('merged')), 'debug', 'deps', 'new.o'), 'x')
    const results: { target: string; skipped?: string }[] = await cleanTargets(rows, recheck())
    expect(results.find((r) => r.target === targetOf(wt('merged')))?.skipped).toBe(
      `盘点之后 ${RECENT_MINUTES} 分钟内有写入，可能正在编译`,
    )
    expect(existsSync(targetOf(wt('merged')))).toBe(true)
  })

  it('只删要清的 target，worktree 本身和别处一个字节不动', async () => {
    const rows = [...(await plan()).values()]
    const results: { ok: boolean }[] = await cleanTargets(rows.filter((r) => r.clean), recheck())
    expect(results.length).toBe(2)
    expect(results.every((r) => r.ok)).toBe(true)

    expect(existsSync(targetOf(wt('merged')))).toBe(false)
    expect(existsSync(targetOf(wt('orphan')))).toBe(false)
    // worktree 还在：签出的文件、src-tauri 目录本身都留着
    expect(existsSync(path.join(wt('merged'), 'README.md'))).toBe(true)
    expect(existsSync(path.join(wt('merged'), 'src-tauri'))).toBe(true)
    for (const name of ['open', 'locked', 'busy', 'nopr', 'recent']) {
      expect(existsSync(targetOf(wt(name))), name).toBe(true)
    }
    expect(existsSync(targetOf(repo))).toBe(true)
  })

  it('删过之后 target 不存在的不再列出', async () => {
    await cleanTargets((await rowsOf()).filter((r) => r.clean), recheck())
    const rows = await plan()
    expect(rows.has('merged')).toBe(false)
    expect(rows.has('orphan')).toBe(false)
  })

  it('不传复查函数时照删（collect 刚跑完、立即删的调用方）', async () => {
    const results: { ok: boolean }[] = await cleanTargets((await rowsOf()).filter((r) => r.clean))
    expect(results.every((r) => r.ok)).toBe(true)
  })
})
