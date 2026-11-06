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
 * 暂存的文件只要落在下面任一清单里就提醒，两份清单独立判断（review 第 2 条：判定是 externallyAdvanced
 * 时分支也可能同时落后基线）：
 * - 判定是 externallyAdvanced：SVN 侧新改动（改、加、删）的文件；
 * - 分支落后基线：分支基线到 `svn/base` 改过的文件（后端 `behindPaths`）。不按「有暂存就提醒」
 *   （review 第 3 条：用应用切到一条老分支、磁盘是它自己的内容时，那样会每次提交都弹）。
 *
 * 路径不分大小写比（Windows：SVN 改过大小写、索引与 wc.db 不一致时不该漏）。只是提醒：用户可能就是要
 * 提交，确认框给「仍然提交」。返回 `null` 不打扰。
 */
export function commitBeforeFollowWarning(
  staged: string[],
  sync: SyncStatus | null,
): CommitGuardModel | null {
  if (!sync || staged.length === 0) return null
  const key = (p: string) => p.replace(/\\/g, '/').toLowerCase()
  const fromSvn = new Set<string>()
  if (sync.verdict.kind === 'externallyAdvanced') {
    const v = sync.verdict
    for (const c of [...v.changed, ...v.added]) fromSvn.add(key(c.path))
    for (const p of v.removed) fromSvn.add(key(p))
  }
  if (sync.headBehindBase) {
    for (const p of sync.behindPaths ?? []) fromSvn.add(key(p))
  }
  const hit = staged.filter((p) => fromSvn.has(key(p))).map((p) => p.replace(/\\/g, '/'))
  if (hit.length === 0) return null
  const shown = hit.slice(0, MAX_LISTED).map((p) => `· ${p}`)
  if (hit.length > MAX_LISTED) shown.push(`…还有 ${hit.length - MAX_LISTED} 个`)
  return {
    title: '先跟进再提交',
    message:
      `暂存的这些文件在 SVN 上有当前分支还没跟进的新内容：\n${shown.join('\n')}\n\n` +
      '建议先 Cancel，点顶部横幅上的「跟进」，再提交你自己的改动。',
    warning:
      '先提交的话，SVN 带进来的内容会被记成你的提交；SVN 之后再改同一处，跟进就会冲突，甚至静默合错。',
    confirmLabel: '仍然提交',
  }
}
