import type { FileHistoryTab, FileHistoryTarget } from '@/engine/fileHistory'
import type { MenuItem } from './ContextMenu'

/**
 * 文件右键菜单里的「File History」「Blame」两项，排在既有菜单项前面。
 *
 * 不传 `open` 时一项都不加（组件单独挂载、测试）。`disabledReason` 非空时两项置灰，
 * 标签后面带上原因——比直接不给更能说明「为什么这里不行」。
 */
export function fileHistoryItems(
  open: ((target: FileHistoryTarget) => void) | undefined,
  makeTarget: (tab: FileHistoryTab) => FileHistoryTarget,
  disabledReason: string | null = null,
): MenuItem[] {
  if (!open) return []
  const suffix = disabledReason ? `（${disabledReason}）` : ''
  return [
    {
      label: `File History${suffix}`,
      onSelect: () => open(makeTarget('history')),
      disabled: disabledReason !== null,
    },
    {
      label: `Blame${suffix}`,
      onSelect: () => open(makeTarget('blame')),
      disabled: disabledReason !== null,
    },
  ]
}
