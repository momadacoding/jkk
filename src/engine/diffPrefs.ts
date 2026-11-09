/**
 * diff 面板的一排开关（参照 Fork）：忽略换行符、忽略空白、不可见字符、自动换行、上下文行数、
 * 整个文件、左右分栏。全局一份，所有标签、所有 diff 视图共用。
 *
 * 哪些开关要后端重读、哪些只是换个画法，见 `backendDiffOptions`。
 */
export interface DiffPrefs {
  /** 左右分栏（旧版在左、新版在右）；关着是上下合在一栏。 */
  split: boolean
  /** 只差行尾 `\r` 的一删一增当作没改（前端后处理，见 `diffEol.ts`）。默认开。 */
  ignoreEol: boolean
  /** 忽略所有空白的改动（libgit2 `IGNORE_WHITESPACE`，同 `git diff -w`）。 */
  ignoreWhitespace: boolean
  /** 空格、制表符、行尾 `\r` 画成可见的符号。 */
  showInvisibles: boolean
  /** 长行折到下一行显示，不横向滚动。 */
  wrap: boolean
  /** 每个 hunk 前后的上下文行数，取 `CONTEXT_STEPS` 里的值。 */
  contextLines: number
  /** 显示整个文件：上下文不限行数。开着时 `contextLines` 不起作用。 */
  fullFile: boolean
}

export const DEFAULT_DIFF_PREFS: DiffPrefs = {
  split: false,
  ignoreEol: true,
  ignoreWhitespace: false,
  showInvisibles: false,
  wrap: false,
  contextLines: 3,
  fullFile: false,
}

/** 「减少 / 增加上下文」按一下走一档。3 是 git 的默认值。 */
export const CONTEXT_STEPS: readonly number[] = [0, 1, 3, 5, 10, 25, 50, 100]

/** 与 Rust 侧 `engine::diff::FULL_FILE_CONTEXT` 相同，理由见那里。 */
export const FULL_FILE_CONTEXT = 1 << 24

export const DIFF_PREFS_KEY = 'mygitclient:diff-prefs'

/** 下一档上下文行数；已经到头时返回 null（按钮禁用）。不在档位上的值按最近的档位走。 */
export function stepContext(current: number, dir: 1 | -1): number | null {
  if (dir > 0) return CONTEXT_STEPS.find((n) => n > current) ?? null
  const lower = CONTEXT_STEPS.filter((n) => n < current)
  return lower.length > 0 ? lower[lower.length - 1] : null
}

/** 读存档：坏的、缺的字段一律回到默认，不让一份旧存档把面板弄坏。 */
export function parseDiffPrefs(raw: string | null): DiffPrefs {
  if (!raw) return { ...DEFAULT_DIFF_PREFS }
  let data: unknown
  try {
    data = JSON.parse(raw)
  } catch {
    return { ...DEFAULT_DIFF_PREFS }
  }
  if (typeof data !== 'object' || data === null) return { ...DEFAULT_DIFF_PREFS }
  const o = data as Record<string, unknown>
  const flag = (k: keyof DiffPrefs): boolean =>
    typeof o[k] === 'boolean' ? (o[k] as boolean) : (DEFAULT_DIFF_PREFS[k] as boolean)
  const ctx = o.contextLines
  return {
    split: flag('split'),
    ignoreEol: flag('ignoreEol'),
    ignoreWhitespace: flag('ignoreWhitespace'),
    showInvisibles: flag('showInvisibles'),
    wrap: flag('wrap'),
    contextLines: typeof ctx === 'number' && CONTEXT_STEPS.includes(ctx) ? ctx : DEFAULT_DIFF_PREFS.contextLines,
    fullFile: flag('fullFile'),
  }
}

/** 传给 `read_diff` / `commit_file_diff` 的 `options`（Rust `DiffViewOptions`）。 */
export interface BackendDiffOptions {
  contextLines: number
  ignoreWhitespace: boolean
}

/** 只有这两项要后端重读；其余开关只是换个画法，切换时不发请求。 */
export function backendDiffOptions(prefs: DiffPrefs): BackendDiffOptions {
  return {
    contextLines: prefs.fullFile ? FULL_FILE_CONTEXT : prefs.contextLines,
    ignoreWhitespace: prefs.ignoreWhitespace,
  }
}

/** 后端选项的字符串键：store 拿它判断在跑的那一轮读的是不是同一份，effect 拿它做依赖。 */
export function diffOptionsKey(opts: BackendDiffOptions): string {
  return `${opts.contextLines}:${opts.ignoreWhitespace ? 1 : 0}`
}

/** 前端重切 hunk 用的上下文行数：整个文件时不限。 */
export function effectiveContext(prefs: DiffPrefs): number {
  return prefs.fullFile ? Number.POSITIVE_INFINITY : prefs.contextLines
}
