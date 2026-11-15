import { basename, normalizePath } from './path'

/**
 * 「自动跟进被挡下」的提醒（2026-11-11 使用报告 9.5）的纯逻辑：各标签上报自己的原因，
 * 这里汇总成一条文字，变了才交给后端（任务栏角标、托盘图标上的点、托盘提示）。
 *
 * 原因只写在窗口里的横幅上时，人不在看这个窗口就一直不知道。提醒跟着横幅上的原因行走：
 * 原因行在就挂着，原因行没了就撤掉。
 */

/** 一个被挡下的仓库：打开时的路径（给人看的名字从它取）和横幅上那行原因。 */
export interface BlockedRepo {
  path: string
  reason: string
}

/**
 * 汇总成托盘提示里的那一行。没有被挡下的仓库时是 `null`。
 *
 * 几个仓库同时被挡下时只写名字：托盘提示在 Windows 上最多 127 个字符，几条原因放不下，
 * 具体的点开窗口看横幅。
 */
export function describeAttention(blocked: ReadonlyMap<string, BlockedRepo>): string | null {
  const repos = [...blocked.values()]
  if (repos.length === 0) return null
  if (repos.length === 1) return `${basename(repos[0].path)} 没有自动跟进：${repos[0].reason}`
  const names = displayNames(repos.map((r) => r.path))
  return `${repos.length} 个仓库没有自动跟进：${names.join('、')}`
}

/**
 * 给人看的仓库名：一般是最后一级目录；同名的（同一个工程的几份工作副本，trunk 和分支各一份）
 * 多带一级，否则托盘上写「Client、Client」分不出是哪个（独立 review 第 8 条）。
 */
function displayNames(paths: string[]): string[] {
  const short = paths.map(basename)
  const seen = new Map<string, number>()
  for (const name of short) seen.set(name.toLowerCase(), (seen.get(name.toLowerCase()) ?? 0) + 1)
  return paths.map((p, i) => {
    if ((seen.get(short[i].toLowerCase()) ?? 0) < 2) return short[i]
    const parts = p.replace(/\\/g, '/').replace(/\/+$/, '').split('/')
    return parts.slice(-2).join('/')
  })
}

export interface AttentionTracker {
  /** 一个标签上报自己现在的原因。`null` 或空白 = 没有被挡下（包括标签关掉时）。 */
  report(path: string, reason: string | null): void
}

/**
 * `send(reason, flash)`：`reason` 为 `null` 撤掉提醒；`flash` 让任务栏按钮闪一次。
 *
 * 汇总出来的文字没变、也没有新的事就不发：标签每次渲染都会上报。只在**多了一件事**时闪：新的仓库被挡下，
 * 或者同一个仓库换了原因。别的仓库撤掉之后剩下的那条不是新的事，不闪。
 *
 * 有新的事时文字相同也要发：几个仓库同时被挡下时托盘上只写名字，其中一个换了原因，汇总文字不变，
 * 按文字去重就不闪了（独立 review 第 3 条）。
 */
export function createAttentionTracker(
  send: (reason: string | null, flash: boolean) => void,
): AttentionTracker {
  const blocked = new Map<string, BlockedRepo>()
  let sent: string | null = null
  return {
    report(path, reason) {
      const key = normalizePath(path)
      const text = reason?.trim() ?? ''
      const before = blocked.get(key)?.reason
      if (text) blocked.set(key, { path, reason: text })
      else blocked.delete(key)
      const next = describeAttention(blocked)
      const flash = text !== '' && text !== before
      if (next === sent && !flash) return
      sent = next
      send(next, flash)
    },
  }
}
