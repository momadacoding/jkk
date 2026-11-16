import type { ConflictInfo, FileChange } from './types'

/**
 * 冲突文件排到最前，其余保持原顺序。
 *
 * 冲突文件和普通改动一起按路径混排时，改动一多就淹没在列表中间，
 * 而它们恰恰是 merge / rebase 停下来之后唯一必须先处理的东西。
 * 没有冲突时返回原数组，免得每次刷新都换一个新引用。
 */
export function conflictsFirst(files: FileChange[]): FileChange[] {
  if (!files.some((f) => f.status === 'conflicted')) return files
  return [
    ...files.filter((f) => f.status === 'conflicted'),
    ...files.filter((f) => f.status !== 'conflicted'),
  ]
}

export function countConflicts(files: FileChange[]): number {
  return files.filter((f) => f.status === 'conflicted').length
}

/**
 * 顶部横幅文案；`null` 表示不显示。
 *
 * 只说「merge in progress」不说还剩几个冲突，用户得自己去列表里找——
 * 横幅本来就是最显眼的位置，把数字放在这里。
 */
export function repoStateBanner(state: string, conflicts: number): string | null {
  if (state === 'normal') return null
  const sequenceOf = SEQUENCES[state]
  const head = sequenceOf ? `${sequenceOf} in progress（一次处理多个提交）` : `${state} in progress`
  if (conflicts > 0) return `${head} · ${conflicts} 个文件有冲突`
  // 第 74 条：冲突都解决了、Staged 又是空的时候，界面上只剩这一行，用户会以为没事可做。
  // 说「没有未解决的冲突」而不是「已全部解决」：`merge --no-commit` 停下时本来就没有冲突
  const name = FINISHED_BY_COMMIT[state]
  return name ? `${head} · 没有未解决的冲突，提交以完成${spaced(name)}` : head
}

/**
 * 提交一次就能收尾的操作，值是给人看的名字。
 * rebase 与一次处理多个提交（sequencer）的收尾都是 `--continue`（第 51 条），不在此列。
 */
const FINISHED_BY_COMMIT: Record<string, string> = { merge: '合并', 'cherry-pick': 'cherry-pick', revert: 'revert' }

/** `repo_state` 对一次处理多个提交的报法，值是对应的单个操作名（后端 `engine::repo::state`）。 */
const SEQUENCES: Record<string, string> = { 'cherry-pick-sequence': 'cherry-pick', 'revert-sequence': 'revert' }

/** 中文紧挨着中文，英文词前空一格：「提交合并」「提交 cherry-pick」。 */
function spaced(name: string): string {
  return /^[a-z]/i.test(name) ? ` ${name}` : name
}

/** 提交按钮的状态。纯逻辑，组件只负责渲染。 */
export interface CommitGate {
  /** 按钮文字 */
  label: string
  /** 非 null 时按钮与 Ctrl+Enter 都不可用，这句话显示在按钮下方 */
  blocked: string | null
  /** 可以提交、但需要额外说明的情形（Staged 为空） */
  note: string | null
}

/**
 * 第 74 条：合并 / cherry-pick / revert 进行中的提交流程。
 *
 * - 还有未解决的冲突：拦下。git 也会拒绝，但只给英文原文。不看 state：
 *   `stash pop` 撞出的冲突不留任何进行中的状态，提交照样会被 git 拒绝
 * - 冲突都解决了：按钮改叫「提交合并」等，让人知道提交这一下就是在收尾
 * - Staged 为空：仍然能提交（后端对单个 cherry-pick / revert 带 `--allow-empty`），说明为什么
 * - 勾了 Amend：merge / cherry-pick / revert 进行中 git 拒绝 `--amend`，只给英文原文，这里先拦。
 *   rebase 不拦：停在 edit 上改提交正是它的用法
 *
 * 一次处理多个提交（`*-sequence`）时提交只了结当前这一个，按钮与说明都不改。
 *
 * @param state `repo_state` 的结果
 * @param conflicts Unstaged 区里的冲突文件数
 * @param staged Staged 区的文件数
 * @param amend Amend 勾选框
 */
export function commitGate(state: string, conflicts: number, staged: number, amend = false): CommitGate {
  const name = FINISHED_BY_COMMIT[state]
  const label = name ? `提交${spaced(name)}` : 'Commit'
  if (conflicts > 0) {
    return { label, blocked: `还有 ${conflicts} 个冲突文件未解决，解决并暂存之后才能提交`, note: null }
  }
  const amendRefused = name ? state : SEQUENCES[state]
  if (amend && amendRefused) {
    return { label, blocked: `${amendRefused} 进行中不能 Amend，取消勾选后再提交`, note: null }
  }
  let note: string | null = null
  if (staged === 0 && state === 'merge') {
    note = 'Staged 为空也可以提交：合并提交记下的是「这两条历史已经合到一起」。'
  } else if (staged === 0 && name) {
    note = `结果与当前 HEAD 相同，提交会记一个没有改动的${spaced(name)} 提交。`
  }
  return { label, blocked: null, note }
}

/** 暂存前的冲突标记确认框文案。纯逻辑，组件只负责弹出来。 */
export interface StageConflictConfirmModel {
  title: string
  message: string
  confirmLabel: string
  danger: true
  focusCancel: true
}

function listPaths(heading: string, paths: string[]): string {
  return [heading, ...paths.map((p) => `  ${p}`)].join('\n')
}

/**
 * 要暂存的路径里有还留着冲突标记的冲突文件时，返回确认框文案；不用拦时返回 null。
 *
 * git add 不检查冲突标记，暂存即「标记为已解决」，紧接着一次 commit 标记就进了历史
 * （第 85 条）。所以所有 Stage 入口都要先过这里。
 *
 * 判据只用 `read_conflicts` 已有的分类，不在前端重新扫文件：
 * - `markers`：拦
 * - 冲突信息还没读到（`conflicts` 为 null），或读到的那份里没有这个文件
 *   （status 比冲突信息新）：也拦。没读到不等于没有标记，放行就是把未知当安全
 * - `noMarkers`、二进制冲突、一边删一边改、结构冲突：不拦，冲突说明里已给出处理方式
 *
 * 确认框是整批的：取消后这一批都不暂存，不会只暂存其中的普通文件。默认按钮是取消。
 *
 * @param paths 这次要暂存的全部路径
 * @param unstaged 当前 Unstaged 区（冲突文件只会出现在这里）
 * @param conflicts 冲突分类；null 表示还没读到
 */
export function describeStageConflicts(
  paths: string[],
  unstaged: FileChange[],
  conflicts: ConflictInfo[] | null,
): StageConflictConfirmModel | null {
  if (paths.length === 0) return null
  // Set / Map：Stage All 几万个文件时 includes / find 是平方级，弹框前会卡住界面
  const picked = new Set(paths)
  const byPath = new Map((conflicts ?? []).map((c) => [c.path, c]))
  const markers: string[] = []
  const unknown: string[] = []
  for (const f of unstaged) {
    if (f.status !== 'conflicted' || !picked.has(f.path)) continue
    const info = byPath.get(f.path)
    if (!info) unknown.push(f.path)
    else if (info.case === 'markers') markers.push(f.path)
  }
  const flagged = markers.length + unknown.length
  if (flagged === 0) return null

  const title = flagged === 1
    ? markers.length === 1
      ? `${markers[0]} 里还有冲突标记`
      : `无法确认 ${unknown[0]} 里有没有冲突标记`
    : `${flagged} 个文件可能还有冲突标记`
  const sections = [
    '暂存即标记为已解决，冲突标记会随下一次提交进入历史。仍然暂存？',
    markers.length > 0 ? listPaths(`还有冲突标记（${markers.length}）：`, markers) : null,
    unknown.length > 0 ? listPaths(`还没读到冲突信息，无法确认（${unknown.length}）：`, unknown) : null,
    // 不说「选中的」：Stage All 并没有选中什么
    paths.length > flagged ? `取消的话，这一批 ${paths.length} 个文件都不暂存。` : null,
  ]
  return {
    title,
    message: sections.filter((s): s is string => s !== null).join('\n\n'),
    confirmLabel: '仍然暂存',
    danger: true,
    focusCancel: true,
  }
}
