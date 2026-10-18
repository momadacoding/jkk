/** 一个外部 diff / merge 工具，与 Rust 侧 `ExternalTool` 一一对应。 */
export interface ExternalTool {
  id: string
  label: string
  program: string
  /** 缺省表示这个工具不能做 diff，不出现在 diff 菜单里。 */
  diffArgs?: string[]
  /** 缺省表示这个工具不能做 merge，不出现在 merge 菜单里。 */
  mergeArgs?: string[]
}

/** 按文件名选工具的一条规则；没写的那一项回落到顶层默认值。 */
export interface ToolRule {
  patterns: string[]
  diff?: string
  merge?: string
}

/** `externalTools.json` 的内容，与 Rust 侧 `ExternalToolsConfig` 一一对应。 */
export interface ExternalToolsConfig {
  version: number
  defaultDiff: string | null
  defaultMerge: string | null
  rules: ToolRule[]
  tools: ExternalTool[]
}

export type ToolKind = 'diff' | 'merge'

/**
 * 文件名是否匹配模式：只支持 `*`（任意多个字符）和 `?`（恰好一个字符），不区分大小写。
 *
 * 不转成正则：模式是用户手写的，`[`、`+`、`.` 这些都得按字面量对待，
 * 自己写一个回溯匹配器比逐个转义更不容易漏。按码点切分，`?` 才能对上一个中文字或 emoji。
 */
export function matchesPattern(pattern: string, fileName: string): boolean {
  const p = [...pattern.toLowerCase()]
  const s = [...fileName.toLowerCase()]
  let pi = 0
  let si = 0
  // 最近一个 `*` 的位置，以及它当时对齐到的文本位置，失配时从这里回溯
  let star = -1
  let starSi = 0
  while (si < s.length) {
    if (pi < p.length && (p[pi] === '?' || (p[pi] !== '*' && p[pi] === s[si]))) {
      pi++
      si++
    } else if (pi < p.length && p[pi] === '*') {
      star = pi
      starSi = si
      pi++
    } else if (star >= 0) {
      // 让上一个 `*` 多吞一个字符再试
      pi = star + 1
      starSi++
      si = starSi
    } else {
      return false
    }
  }
  while (pi < p.length && p[pi] === '*') pi++
  return pi === p.length
}

/** 只取文件名部分：规则不看目录，Windows 反斜杠路径同样处理。 */
function baseName(filePath: string): string {
  const parts = filePath.split(/[\\/]/)
  return parts[parts.length - 1] ?? ''
}

function argsFor(tool: ExternalTool, kind: ToolKind): string[] | undefined {
  return kind === 'diff' ? tool.diffArgs : tool.mergeArgs
}

/**
 * 这个文件该用哪个工具：第一条命中的规则说了算，规则没写这一项就用默认工具。
 *
 * 选中的工具做不了这件事（缺对应参数）或找不到时返回 null，**不换别的工具**：
 * 悄悄回退会把场景文件交给 BC 做文本合并，正是规则要避免的。
 * 改名的文件由调用方传新路径。
 */
export function toolFor(
  cfg: ExternalToolsConfig,
  filePath: string,
  kind: ToolKind,
): ExternalTool | null {
  const name = baseName(filePath)
  const rule = cfg.rules.find((r) => r.patterns.some((p) => matchesPattern(p, name)))
  const id = rule?.[kind] ?? (kind === 'diff' ? cfg.defaultDiff : cfg.defaultMerge)
  if (!id) return null
  const tool = cfg.tools.find((t) => t.id === id)
  return tool && argsFor(tool, kind) ? tool : null
}

/** 右键菜单：第一项是匹配到的工具，「用其他工具打开 ▸」里是其余能做这件事的工具。 */
export function toolMenu(
  cfg: ExternalToolsConfig,
  filePath: string,
  kind: ToolKind,
): { primary: ExternalTool | null; others: ExternalTool[] } {
  const primary = toolFor(cfg, filePath, kind)
  const others = cfg.tools.filter((t) => t !== primary && argsFor(t, kind) !== undefined)
  return { primary, others }
}
