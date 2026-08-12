import type { CommitNode, RefPointer } from '../engine/types'
import { sortTags, tagShortName } from '../engine/tagOrder'
import { ContextMenu, useContextMenu } from './ContextMenu'
import { SidebarSection } from './SidebarSection'

interface TagViewProps {
  /** 只传 `kind === 'tag'` 的引用，排序在组件内部做。 */
  tags: RefPointer[]
  /** 排序要按目标提交的时间，所以需要当前图里的提交。 */
  commits: CommitNode[]
  /** 当前选中的 tag 短名，用来高亮。 */
  selectedTag: string | null
  onSelect: (name: string, hash: string) => void
  onCheckout: (name: string) => void
  onPush: (name: string) => void
}

function TagIcon() {
  return (
    <svg className="sb-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <path d="M7.2 2.5H2.5v4.7l6.3 6.3 4.7-4.7z" />
      <circle cx="5" cy="5" r="0.9" />
    </svg>
  )
}

export function TagView({ tags, commits, selectedTag, onSelect, onCheckout, onPush }: TagViewProps) {
  const { menu, open, close } = useContextMenu()
  const sorted = sortTags(tags, commits)

  // 不传 defaultOpen：SidebarSection 只在挂载时读一次这个值，而本组件挂载时
  // refs 还没加载完，`sorted.length > 0` 恒为 false，分区会永远收起来。
  // 跟 BRANCHES 一样默认展开，空的时候显示 "No tags"。
  return (
    <SidebarSection title="TAGS" count={sorted.length}>
      {sorted.length === 0 && <div className="sb-empty">No tags</div>}
      {sorted.map((ref) => {
        const name = tagShortName(ref.name)
        const items = [
          { label: 'Checkout', onSelect: () => onCheckout(name) },
          { label: 'Push Tag', onSelect: () => onPush(name) },
        ]
        return (
          <div
            key={ref.name}
            className={name === selectedTag ? 'sb-row selected' : 'sb-row'}
            title={`${name}\n单击定位 · 双击 checkout（游离 HEAD） · 右键更多操作`}
            onClick={() => onSelect(name, ref.hash)}
            onDoubleClick={() => onCheckout(name)}
            onContextMenu={(e) => open(e, items)}
          >
            <TagIcon />
            <span className="sb-label">{name}</span>
          </div>
        )
      })}
      <ContextMenu menu={menu} onClose={close} />
    </SidebarSection>
  )
}
