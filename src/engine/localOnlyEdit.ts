import { shortHash } from './commitActions'
import type { CommitNode } from './types'

/**
 * 事后改 Local-Only 标记（2026-11-11 使用报告 9.2）的纯逻辑：确认框文案、结局提示、拆分选中项的校验。
 * 后端是 `svn_local_only_edit`，设计见 `docs/superpowers/plans/2026-11-15-local-only-toggle-decisions.md`。
 */

/** 后端 `svn_local_only_edit::EditOutcome`。 */
export interface LocalOnlyEditOutcome {
  branch: string
  /** 改写前的分支末端：`git reset --keep <它>` 能回去。 */
  oldTip: string
  tip: string
  /** 被改的那个 commit 改写之后的 OID（拆分时是留下的 Local-Only 那一个）。图上选中它。 */
  commit: string
  /** 别的本地分支上也有这个 commit：它们没动，留着旧的。 */
  otherBranches: string[]
}

/** 改写的代价，确认框和拆分弹框共用一份，免得两处说出两种话。 */
export const LOCAL_ONLY_EDIT_WARNING =
  '会改写当前分支上这个 commit 和它之后的 commit（内容不变，只是换成新的 commit），' +
  '工作区和暂存区的文件一个都不动。别的分支上如果也有这个 commit，它们不跟着改。'

/** 取消 / 加上标记的确认框。 */
export function describeLocalOnlyToggle(
  commit: CommitNode,
  on: boolean,
): { title: string; message: string; warning: string; confirmLabel: string } {
  const subject = commit.message.split('\n')[0]
  const which = `${shortHash(commit.id)} ${subject}`
  return on
    ? {
        title: '标为「仅本地」',
        message: `把 ${which} 标成「仅本地，不交付」：交付时它会被摘掉，留在你的分支上。`,
        warning: LOCAL_ONLY_EDIT_WARNING,
        confirmLabel: '标为仅本地',
      }
    : {
        title: '取消「仅本地」',
        message: `取消 ${which} 的「仅本地」：之后交付时它会和别的 commit 一起提交上 SVN。`,
        warning: LOCAL_ONLY_EDIT_WARNING,
        confirmLabel: '取消仅本地',
      }
}

/** 改写完成之后要不要说一声：别的分支上也有这个 commit 时说，其余时候不打扰。 */
export function describeLocalOnlyEditDone(
  o: LocalOnlyEditOutcome,
): { title: string; message: string } | null {
  if (o.otherBranches.length === 0) return null
  return {
    title: '别的分支没有跟着改',
    message:
      `${o.otherBranches.join('、')} 上也有这个 commit，它们留着旧的（标记还是改之前的样子）。` +
      `只有当前分支 ${o.branch} 改了。`,
  }
}

/**
 * 拆分弹框里的选中项能不能拆；能拆时为 `null`。与后端 `split_local_only` 的校验一致：
 * 一个都没选、全选了（等于取消标记）都不行。
 */
export function splitSelectionProblem(total: number, chosen: number): string | null {
  if (total < 2) return '这个 commit 只改了一个文件，没法拆。要让它上服务器，用「取消仅本地」。'
  if (chosen === 0) return '选出要上服务器的文件。'
  if (chosen === total) return '全选了：那等于取消「仅本地」，请用那一项。'
  return null
}
