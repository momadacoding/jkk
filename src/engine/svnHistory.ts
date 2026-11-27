/**
 * 混合仓库的文件历史 / Blame 补全 SVN 的纯逻辑（计划 2026-11-27-svn-file-history）。
 *
 * 数据形状与 `src-tauri/src/engine/svn_file_history.rs` 一一对应；Rust 的 `Option` 到这边是 `null`。
 */
import { UNCOMMITTED, type BlameData, type FileHistoryEntry } from './fileHistory'

/** 一次 Blame 最多映射几组，与后端 `MAX_OVERLAY_TARGETS` 一致。 */
export const MAX_OVERLAY_TARGETS = 8

export interface GroupSpec {
  id: string
  path: string
  fileRev: number
}

export interface Cursor {
  pegPath: string
  pegRev: number
  nextTop: number
}

export interface SvnRevEntry {
  rev: number
  author: string
  /** ISO8601 原样。 */
  date: string
  message: string
}

export interface SvnFetchError {
  /** `svn_unreachable` / `svn_auth` / `svn_path_missing` / `svn_cli` */
  code: string
  message: string
}

export interface SvnHistoryPage {
  groups: Record<string, SvnRevEntry[]>
  cursor: Cursor | null
  error: SvnFetchError | null
}

/** 基线提交 id → 已加载的 SVN 提交，从新到旧。 */
export type SvnGroups = Record<string, SvnRevEntry[]>

export interface OverlayTarget {
  baselineId: string
  origPath: string
  fileRev: number
}

export interface BlameRevAuthor {
  rev: number
  author: string
  date: string
}

export type OverlayResult =
  | { kind: 'mapped'; lines: Array<BlameRevAuthor | null> }
  | { kind: 'mismatch'; svnLines: number; gitLines: number }
  | { kind: 'failed'; error: SvnFetchError }
  | { kind: 'skipped' }

/** Blame 一行最终显示谁：git 的归属、SVN 的真实出处，或映射不上时保持「基线 rN」。 */
export type LineSource = { kind: 'git' } | ({ kind: 'svn' } & BlameRevAuthor) | { kind: 'baseline' }

export interface OverlayView {
  /** 与 `BlameData.lines` 同序。 */
  lines: LineSource[]
  /** 本该映射、但没映射上的基线行数（行数对不上、超过上限、没查）。 */
  unmapped: number
  /** 第一个查失败的原因；没有失败为 null。 */
  failure: SvnFetchError | null
}

/** 历史里的基线条目，从新到旧，传给 `svn_file_history`。 */
export function groupSpecs(entries: FileHistoryEntry[]): GroupSpec[] {
  const out: GroupSpec[] = []
  for (const e of entries) {
    const b = e.svn?.baseline
    if (b) out.push({ id: e.id, path: e.path, fileRev: b.fileRev })
  }
  return out
}

/**
 * 已由本地提交代表的交付 revision：基线记的 `deliveredRev`，加上本地提交的「已交付 rN」。
 * 只取前者不够——交付录的基线树与交付的提交相同，`git log --follow` 常常不列它（决策日志 Task 5）。
 */
export function hiddenRevs(entries: FileHistoryEntry[]): number[] {
  const out = new Set<number>()
  for (const e of entries) {
    const d = e.svn?.baseline?.deliveredRev
    if (d != null) out.add(d)
    const s = e.svn?.delivered
    if (s?.kind === 'delivered') out.add(s.rev)
  }
  return [...out]
}

/** 把新的一页并进已加载的分组：按组追加、按 rev 去重。返回新对象。 */
export function mergePage(prev: SvnGroups, page: SvnHistoryPage): SvnGroups {
  const next: SvnGroups = { ...prev }
  for (const [id, entries] of Object.entries(page.groups)) {
    const have = next[id] ?? []
    const seen = new Set(have.map((e) => e.rev))
    next[id] = [...have, ...entries.filter((e) => !seen.has(e.rev))]
  }
  return next
}

const targetKey = (commit: string, path: string) => `${commit}\u001f${path}`

/** Blame 里归到基线的行按 (基线, 路径) 聚合，按行数从多到少取前 `MAX_OVERLAY_TARGETS` 组。 */
export function overlayTargets(blame: BlameData): OverlayTarget[] {
  const counts = new Map<string, { t: OverlayTarget; n: number }>()
  for (const l of blame.lines) {
    if (l.commit === UNCOMMITTED) continue
    const b = blame.commits[l.commit]?.baseline
    if (!b) continue
    const k = targetKey(l.commit, l.origPath)
    const hit = counts.get(k)
    if (hit) hit.n++
    else counts.set(k, { t: { baselineId: l.commit, origPath: l.origPath, fileRev: b.fileRev }, n: 1 })
  }
  return [...counts.values()]
    .sort((a, b) => b.n - a.n)
    .slice(0, MAX_OVERLAY_TARGETS)
    .map((x) => x.t)
}

/** 把 overlay 的结果按行套到 Blame 上。 */
export function applyOverlay(blame: BlameData, targets: OverlayTarget[], results: OverlayResult[]): OverlayView {
  const byKey = new Map<string, OverlayResult>()
  targets.forEach((t, i) => {
    const r = results[i]
    if (r) byKey.set(targetKey(t.baselineId, t.origPath), r)
  })
  let unmapped = 0
  let failure: SvnFetchError | null = null
  const lines = blame.lines.map((l): LineSource => {
    if (l.commit === UNCOMMITTED || !blame.commits[l.commit]?.baseline) return { kind: 'git' }
    const r = byKey.get(targetKey(l.commit, l.origPath))
    if (r?.kind === 'mapped') {
      const who = r.lines[l.origLine - 1]
      if (who) return { kind: 'svn', ...who }
      unmapped++
      return { kind: 'baseline' }
    }
    if (r?.kind === 'failed') {
      failure ??= r.error
      return { kind: 'baseline' }
    }
    unmapped++
    return { kind: 'baseline' }
  })
  return { lines, unmapped, failure }
}
