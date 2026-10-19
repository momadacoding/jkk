/** 一个外部 diff / merge 工具，与 Rust 侧 `ExternalTool` 一一对应。 */
export interface ExternalTool {
  id: string
  label: string
  program: string
  /** 缺省表示这个工具不能做 diff，不出现在 diff 菜单里。 */
  diffArgs?: string[]
  /** 缺省表示这个工具不能做 merge，不出现在 merge 菜单里。 */
  mergeArgs?: string[]
  /** 哪些退出码算正常结束；缺省只认 0。判断在后端做，前端只读事件里的结果。 */
  successExitCodes?: number[]
  /** 已知故障码的说明，键是十进制退出码。 */
  exitCodeMessages?: Record<string, string>
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

/** 外部 diff 的两侧，与 Rust 侧 `DiffTarget` 一一对应（`kind` 做标签）。 */
export type DiffTarget =
  | { kind: 'unstaged'; file: string }
  | { kind: 'staged'; file: string }
  | { kind: 'commit'; oid: string; file: string; oldFile: string | null }
  | { kind: 'range'; from: string; to: string; file: string; oldFile: string | null }

/** 每个外部工具菜单的最后一项（spec §4.5）。 */
export const EDIT_CONFIG_LABEL = '编辑外部工具配置…'
/** 配了不止一个可用工具时的子菜单标题。 */
export const OTHER_TOOLS_LABEL = '用其他工具打开 ▸'

export interface ToolEntry {
  label: string
  toolId: string
}

/**
 * 外部 diff 菜单里的工具项：第一项「在 X 中对比」，其余进子菜单只写工具名。
 * 配置读不出来时一项都没有，调用方只留「编辑外部工具配置…」。
 */
export function diffMenuEntries(
  cfg: ExternalToolsConfig | null,
  filePath: string,
): { primary: ToolEntry | null; others: ToolEntry[] } {
  if (!cfg) return { primary: null, others: [] }
  const { primary, others } = toolMenu(cfg, filePath, 'diff')
  return {
    primary: primary ? { label: `在 ${primary.label} 中对比`, toolId: primary.id } : null,
    others: others.map((t) => ({ label: t.label, toolId: t.id })),
  }
}

/**
 * 提交详情的 diff scope 转成外部 diff 的目标。
 *
 * 只认 `commit:<oid>` 和 `range:<a>..<b>`（与 Rust 侧 `read_diff` 的解析一致），
 * 其余（`workdir`、解析不了的）返回 null，菜单里就不给对比项。
 */
export function diffTargetFromScope(
  scope: string | null,
  file: string,
  oldFile: string | null,
): DiffTarget | null {
  if (!scope) return null
  if (scope.startsWith('commit:')) {
    const oid = scope.slice('commit:'.length)
    return oid ? { kind: 'commit', oid, file, oldFile } : null
  }
  if (scope.startsWith('range:')) {
    const range = scope.slice('range:'.length)
    const at = range.indexOf('..')
    if (at < 0) return null
    const from = range.slice(0, at)
    const to = range.slice(at + 2)
    return from && to ? { kind: 'range', from, to, file, oldFile } : null
  }
  return null
}

/** 判断一次外部工具退出要不要报错所需的信息，取自 `external-tool-exited` 事件。 */
export interface ToolExitInfo {
  toolLabel: string
  code: number | null
  error: string | null
  /** 后端按这个工具的 `successExitCodes` 判断的结果。 */
  success: boolean
  /** 配置里给这个退出码写的说明。 */
  codeMessage: string | null
}

/**
 * 外部工具退出后要不要报给用户：正常结束返回 null，否则返回一句带工具名和退出码的说明。
 *
 * 「正常」由后端按工具配置判断，这里不再自己看退出码是不是 0：BC 用 13 表示「有差异」。
 * 退出码只用来报告工具故障，**永远不用来判断冲突是否已解决**。
 */
export function describeToolExit(exit: ToolExitInfo): string | null {
  const { toolLabel, code, error } = exit
  if (error) return `${toolLabel}：${error}`
  if (exit.success) return null
  if (code === null) return `${toolLabel} 被异常终止`
  if (exit.codeMessage) return `${toolLabel} 退出码 ${code}：${exit.codeMessage}`
  return `${toolLabel} 退出码 ${code}，可能没有正常完成`
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
