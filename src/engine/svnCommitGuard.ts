import type { SyncStatus } from './svnSync'

/** 提交前「先跟进」提醒的确认框（事故分析 6.6）。 */
export interface CommitGuardModel {
  title: string
  message: string
  warning: string
  confirmLabel: string
}

const MAX_LISTED = 12

/**
 * 提交前要不要提醒「先跟进」（事故分析 6.6）。
 *
 * 2026-11-05 的快照提交就是这么来的：TortoiseSVN 更新之后、点「跟进」之前，SVN 带进来的内容出现在
 * 「本地改动」里，被一起暂存、提交进了 git，成了「你的提交」。之后 SVN 再改同一片代码，跟进就冲突。
 *
 * 两种情形：
 * - 判定是 externallyAdvanced：暂存的文件里有 SVN 侧新改动（改、加、删）的，列出来；
 * - 分支落后基线：「本地改动」是相对旧基线算的，里面混着基线带进来的文件，前端没有这份清单，
 *   有暂存就提醒。
 *
 * 只是提醒：用户可能就是要提交（例如已经确认过内容），确认框给「仍然提交」。返回 `null` 不打扰。
 */
export function commitBeforeFollowWarning(
  staged: string[],
  sync: SyncStatus | null,
): CommitGuardModel | null {
  if (!sync || staged.length === 0) return null
  const norm = (p: string) => p.replace(/\\/g, '/')
  const consequence =
    '先提交的话，SVN 带进来的内容会被记成你的提交；SVN 之后再改同一处，跟进就会冲突，甚至静默合错。'
  if (sync.verdict.kind === 'externallyAdvanced') {
    const v = sync.verdict
    const fromSvn = new Set([
      ...v.changed.map((c) => norm(c.path)),
      ...v.added.map((c) => norm(c.path)),
      ...v.removed.map(norm),
    ])
    const hit = staged.map(norm).filter((p) => fromSvn.has(p))
    if (hit.length === 0) return null
    const shown = hit.slice(0, MAX_LISTED).map((p) => `· ${p}`)
    if (hit.length > MAX_LISTED) shown.push(`…还有 ${hit.length - MAX_LISTED} 个`)
    return {
      title: '先跟进再提交',
      message:
        `暂存的文件里有 SVN 上的新内容，还没跟进：\n${shown.join('\n')}\n\n` +
        '建议先取消，点顶部横幅上的「跟进」，再提交你自己的改动。',
      warning: consequence,
      confirmLabel: '仍然提交',
    }
  }
  if (sync.verdict.kind === 'inSync' && sync.headBehindBase) {
    return {
      title: '先跟进再提交',
      message:
        '当前分支落后基线，「本地改动」里混着基线带进来的文件（它们不是你的改动）。' +
        '建议先取消，点顶部横幅上的「跟进」，再提交你自己的改动。',
      warning: consequence,
      confirmLabel: '仍然提交',
    }
  }
  return null
}
