import {
  diffMenuEntries,
  EDIT_CONFIG_LABEL,
  mergeMenuEntries,
  OTHER_TOOLS_LABEL,
  type DiffTarget,
  type ExternalToolsConfig,
  type ToolEntry,
} from '@/engine/externalTools'
import type { MenuItem } from './ContextMenu'

/** 组件拼外部工具菜单需要的东西，由 `RepoTab` 从 store 里接过来。 */
export interface ExternalToolsMenu {
  /** 配置读取失败时为 null：菜单里只留「编辑外部工具配置…」。 */
  config: ExternalToolsConfig | null
  onDiff: (toolId: string, target: DiffTarget) => void
  /** 用外部工具解决一个冲突文件。 */
  onMerge: (toolId: string, file: string) => void
  onEditConfig: () => void
}

/** 第一项是匹配到的工具，其余进「用其他工具打开 ▸」子菜单，最后是配置入口。 */
function toolItems(
  tools: ExternalToolsMenu,
  entries: { primary: ToolEntry | null; others: ToolEntry[] } | null,
  run: (toolId: string) => void,
): MenuItem[] {
  const items: MenuItem[] = []
  if (entries) {
    const { primary, others } = entries
    if (primary) items.push({ label: primary.label, onSelect: () => run(primary.toolId) })
    if (others.length > 0) {
      items.push({
        label: OTHER_TOOLS_LABEL,
        onSelect: () => {},
        children: others.map((t) => ({ label: t.label, onSelect: () => run(t.toolId) })),
      })
    }
  }
  items.push({ label: EDIT_CONFIG_LABEL, onSelect: tools.onEditConfig })
  return items
}

/**
 * 一个文件的外部 diff 菜单项，排在既有菜单项后面。
 *
 * `target` 为 null 表示这个文件不能对比（冲突文件只提供 merge），只留配置入口。
 * 不传 `tools` 时一项都不加，组件在没接外部工具的地方照旧工作。
 */
export function externalDiffItems(
  tools: ExternalToolsMenu | undefined,
  target: DiffTarget | null,
): MenuItem[] {
  if (!tools) return []
  return toolItems(
    tools,
    target ? diffMenuEntries(tools.config, target.file) : null,
    (toolId) => target && tools.onDiff(toolId, target),
  )
}

/**
 * 冲突文件的外部 merge 菜单项。`file` 为 null 表示这个冲突不能交给外部工具
 * （③、④c、⑤，或冲突信息还没读到），只留配置入口。
 */
export function externalMergeItems(
  tools: ExternalToolsMenu | undefined,
  file: string | null,
): MenuItem[] {
  if (!tools) return []
  return toolItems(
    tools,
    file ? mergeMenuEntries(tools.config, file) : null,
    (toolId) => file && tools.onMerge(toolId, file),
  )
}
