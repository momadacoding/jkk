/**
 * 「更新」（`svnSyncFlow`）与「交付」（`svnSyncOutFlow`）两条流程共用的小块。
 *
 * 原来两边逐行各抄一份（第 77 号 issue）：交付侧修好的问题，更新侧要等单独的 PR 再照抄一遍。
 * 只收两边**完全相同**的部分；各自不同的（更新侧的跟进刷新、交付侧失败时也刷新的 `refreshing`）
 * 留在各自的流程里。
 */
import type { OutcomeNotice } from '../engine/svnSyncIn'
import type { ConfirmRequest } from './ConfirmDialog'

export interface AfterSyncDeps {
  repoPath: string
  reportError: (err: unknown) => void
  refreshSvnSyncState: (path: string) => Promise<void>
  refreshSvnSync: (path: string) => Promise<void>
  fetchSvnLog: (path: string) => Promise<unknown>
}

/**
 * 同步收尾之后的刷新。`repo-changed` 由后端补发，这里只补它不管的 SVN 侧状态。
 *
 * **自己不抛**：它跑在对话框的 `action` 里，同步已经落地之后再抛，对话框会留在原地报错、
 * 按钮还能点，用户以为没做成；失败之后抛的话，后端原文（交付 D6 失败时那是用户唯一知道
 * 「提交已经成功、点继续」的地方）会被顶掉（交付最终 review 第 3 条）。刷新失败只进错误条。
 *
 * 两项同时发、各报各的错。复查必须在命令返回的这一拍里进去，才能与后端补发的 `repo-changed`
 * 共用一轮 `svn status`（见 `SvnSyncFlowDeps.refreshSvnSync` 与 `repoStore` 的 `refreshSvnSync`）；
 * 排在读同步状态后面的话多半要白扫一遍，
 * 同步状态读不出来也不该让复查不跑。
 */
export function createAfterSync(d: AfterSyncDeps): () => Promise<void> {
  return async () => {
    const results = await Promise.allSettled([
      d.refreshSvnSyncState(d.repoPath),
      d.refreshSvnSync(d.repoPath),
    ])
    for (const r of results) if (r.status === 'rejected') d.reportError(r.reason)
    void d.fetchSvnLog(d.repoPath)
  }
}

/**
 * 同一时间只走一个流程。预览要跑 `svn status -u`（网络），那几秒里界面上没有框挡着，
 * 工具栏的按钮和横幅上的按钮都点得到；两个流程的确认框会互相顶替
 * （PR #49 Codex review #5）。后端的锁拦得住第二次执行，拦不住界面的错乱。
 *
 * `running` 由调用方用 `useRef` 持有：更新与交付两个流程共用同一个，互相也挡。
 */
export async function exclusive(
  running: { current: boolean },
  run: () => Promise<void>,
): Promise<void> {
  if (running.current) return
  running.current = true
  try {
    await run()
  } finally {
    running.current = false
  }
}

/** 结局提示。`null`（成功不打扰）时什么都不弹；`notice` 让它只有一个按钮。 */
export async function showOutcomeNotice(
  confirm: (request: ConfirmRequest) => Promise<boolean>,
  n: OutcomeNotice | null,
): Promise<void> {
  if (!n) return
  await confirm({ title: n.title, message: n.detail, confirmLabel: '知道了', notice: n.tone })
}
