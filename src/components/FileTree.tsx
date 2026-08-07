import { useState } from 'react'
import type { TreeNode } from '../engine/fileTree'
import type { FileStatus } from '../engine/types'

/** 状态角标的字形与配色，和 Fork 一致：一眼扫过去就知道这次动了什么。 */
const STATUS_MARK: Record<FileStatus, { label: string; kind: string }> = {
  added: { label: '+', kind: 'added' },
  modified: { label: 'M', kind: 'modified' },
  deleted: { label: '−', kind: 'deleted' },
  renamed: { label: 'R', kind: 'renamed' },
  copied: { label: 'C', kind: 'renamed' },
  conflicted: { label: '!', kind: 'conflicted' },
}

function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      className={open ? 'ft-chevron open' : 'ft-chevron'}
      width="10"
      height="10"
      viewBox="0 0 10 10"
      fill="currentColor"
    >
      <path d="M3 1.5l4 3.5-4 3.5z" />
    </svg>
  )
}

function FolderIcon() {
  return (
    <svg className="ft-icon folder" width="13" height="13" viewBox="0 0 16 16" fill="currentColor">
      <path d="M1.5 3h4l1.2 1.5h7.8v8H1.5z" />
    </svg>
  )
}

function FileIcon() {
  return (
    <svg className="ft-icon" width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.2">
      <path d="M3.5 1.5h6l3 3v10h-9z" />
      <path d="M9.5 1.5v3h3" />
    </svg>
  )
}

interface RowsProps {
  nodes: TreeNode[]
  depth: number
  selectedPath: string | null
  isOpen: (path: string) => boolean
  onToggle: (path: string) => void
  onSelect: (path: string) => void
}

function Rows({ nodes, depth, selectedPath, isOpen, onToggle, onSelect }: RowsProps) {
  return (
    <>
      {nodes.map((node) => {
        // 每层缩进 14px；折叠箭头占的位置在文件行上留空，否则同层的名字对不齐
        const indent = { paddingLeft: 6 + depth * 14 }

        if (node.kind === 'dir') {
          const open = isOpen(node.path)
          return (
            <div key={node.path}>
              <div className="ft-row ft-dir" style={indent} onClick={() => onToggle(node.path)}>
                <Chevron open={open} />
                <FolderIcon />
                <span className="ft-name">{node.name}</span>
              </div>
              {open && (
                <Rows
                  nodes={node.children}
                  depth={depth + 1}
                  selectedPath={selectedPath}
                  isOpen={isOpen}
                  onToggle={onToggle}
                  onSelect={onSelect}
                />
              )}
            </div>
          )
        }

        const mark = node.status ? STATUS_MARK[node.status] : null
        return (
          <div
            key={node.path}
            className={node.path === selectedPath ? 'ft-row ft-file selected' : 'ft-row ft-file'}
            style={indent}
            title={node.path}
            onClick={() => onSelect(node.path)}
          >
            <span className="ft-chevron-spacer" />
            {mark ? (
              <span className={`ft-badge ${mark.kind}`}>{mark.label}</span>
            ) : (
              <span className="ft-badge-spacer" />
            )}
            <FileIcon />
            <span className="ft-name">{node.name}</span>
          </div>
        )
      })}
    </>
  )
}

interface FileTreeProps {
  nodes: TreeNode[]
  selectedPath: string | null
  onSelect: (path: string) => void
  /**
   * 没被用户动过的目录默认是展开还是收起。
   *
   * 变更列表就那么几个文件，全展开才看得到；整仓文件树动辄上万条，
   * 全展开会把渲染拖垮，所以那边传 false。
   */
  defaultOpen?: boolean
}

export function FileTree({ nodes, selectedPath, onSelect, defaultOpen = true }: FileTreeProps) {
  /**
   * 只记「用户改过的那些目录」，没记录的一律走 `defaultOpen`。
   *
   * 直接存一个展开集合的话，切到另一个提交时新出现的目录不在集合里，
   * 整棵树就会是收起的；存覆盖项则天然对新目录生效，也不需要任何同步 effect。
   */
  const [overrides, setOverrides] = useState<Record<string, boolean>>({})

  const isOpen = (path: string) => overrides[path] ?? defaultOpen
  const toggle = (path: string) => setOverrides((cur) => ({ ...cur, [path]: !(cur[path] ?? defaultOpen) }))

  if (nodes.length === 0) return <div className="ft-empty">No files</div>

  return (
    <div className="file-tree">
      <Rows
        nodes={nodes}
        depth={0}
        selectedPath={selectedPath}
        isOpen={isOpen}
        onToggle={toggle}
        onSelect={onSelect}
      />
    </div>
  )
}
