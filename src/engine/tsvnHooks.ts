/**
 * TortoiseSVN 客户端钩子的状态与文案（C1h；分析 §6.13.5）。纯逻辑，组件只负责渲染。
 */

/** 后端 `tsvn_hooks::HookHealth`。 */
export type HookHealth =
  | { state: 'notRegistered' }
  | { state: 'healthy'; blockLeaks: boolean }
  /** 注册过，但不完整或指向别的程序。跟进按没注册处理。 */
  | { state: 'stale'; reasons: string[] }

/** 后端 `tsvn_hooks::HookEntry`：注册表 `hooks` 值里的一条。 */
export interface HookEntry {
  /** 不带 `_hook` 后缀，例如 `post_commit`。 */
  kind: string
  wcPath: string
  command: string
  wait: string
  show: string
  enforce: string
}

/** 后端 `commands::svn::TsvnHooksStatus`。 */
export interface TsvnHooksStatus {
  /** 只有 Windows 上有 TortoiseSVN。 */
  supported: boolean
  health: HookHealth
  /** 勾选「提交前拦截本地补丁」时将写入的全部条目（含 check_commit）。 */
  planned: HookEntry[]
  /** 挡住注册的别人的同类钩子。 */
  conflicts: HookEntry[]
  /** 当前 jkk 程序的路径：命令行里写的就是它。 */
  exe: string
}

export function healthLabel(h: HookHealth): string {
  switch (h.state) {
    case 'notRegistered':
      return '未注册'
    case 'healthy':
      return h.blockLeaks ? '已注册（提交前拦截本地补丁）' : '已注册'
    case 'stale':
      return `已失效：${h.reasons.join('；')}`
  }
}

/**
 * 钩子失效时的横幅。exe 不在了时 TortoiseSVN 很可能把「启动不了」当钩子失败，每次提交都报错
 * （C1h 日志 Review 1 第 5 条），所以要写明，并给出撤销注册的出口。
 */
export function describeStaleHooks(h: HookHealth | null): { title: string; detail: string } | null {
  if (h?.state !== 'stale') return null
  return {
    title: 'TortoiseSVN 钩子已失效',
    detail:
      `${h.reasons[0] ?? ''}。TortoiseSVN 提交时可能报钩子错误；跟进暂按作者 + 快照判定。` +
      '请重新注册，或者撤销注册。',
  }
}

/**
 * 是不是开发构建的 exe：删掉这个目录之后，TortoiseSVN 提交会报钩子错误。只警告不禁止，
 * 开发时要靠它实测。
 */
export function isDevBuildExe(exe: string): boolean {
  const p = exe.replace(/\//g, '\\').toLowerCase()
  return /\\target\\(debug|release)\\/.test(p) || /\\(\.claude\\)?\.?worktrees\\/.test(p)
}

/**
 * 按勾选过滤条目：不拦截时去掉 check_commit。将写入的列表与挡路的冲突都用它：后端的冲突按
 * 「要拦截」算，不拦截时 check_commit 上的冲突不再挡路（后端此时也不写这一类）。
 */
export function entriesToWrite(entries: HookEntry[], blockLeaks: boolean): HookEntry[] {
  return blockLeaks ? entries : entries.filter((e) => e.kind !== 'check_commit')
}
