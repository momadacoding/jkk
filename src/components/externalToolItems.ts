import {
  diffMenuEntries,
  EDIT_CONFIG_LABEL,
  OTHER_TOOLS_LABEL,
  type DiffTarget,
  type ExternalToolsConfig,
} from '@/engine/externalTools'
import type { MenuItem } from './ContextMenu'

/** 组件拼外部工具菜单需要的东西，由 `RepoTab` 从 store 里接过来。 */
export interface ExternalToolsMenu {
  /** 配置读取失败时为 null：菜单里只留「编辑外部工具配置…」。 */
  config: ExternalToolsConfig | null
  onDiff: (toolId: string, target: DiffTarget) => void
  onEditConfig: () => void
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
  const items: MenuItem[] = []
  if (target) {
    const { primary, others } = diffMenuEntries(tools.config, target.file)
    if (primary) items.push({ label: primary.label, onSelect: () => tools.onDiff(primary.toolId, target) })
    if (others.length > 0) {
      items.push({
        label: OTHER_TOOLS_LABEL,
        onSelect: () => {},
        children: others.map((t) => ({ label: t.label, onSelect: () => tools.onDiff(t.toolId, target) })),
      })
    }
  }
  items.push({ label: EDIT_CONFIG_LABEL, onSelect: tools.onEditConfig })
  return items
}
