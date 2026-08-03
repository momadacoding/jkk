# mygitclient 实施计划 #2：Commit Graph 布局与 SVG 渲染

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现自研 commit graph 布局引擎（纯函数）+ SVG 渲染器 + 虚拟滚动与基础交互，达到 Sublime Merge 级别的布局与视觉表现。

**Architecture:** 布局算法与渲染器解耦：布局引擎接收 `CommitNode[]` 输出带 lane 坐标的 `LayoutRow[]`；渲染器接口抽象 SVG/Canvas 实现；MVP 先用 SVG + viewport clipping；交互由 React 组件处理。

**Tech Stack:** React, TypeScript, SVG, Vitest.

## Global Constraints

- 布局算法必须是纯函数：相同输入 → 相同输出，可脱离 React/Tauri 单独测试。
- 行高固定（例如 28px），lane 宽度固定（20px），圆点半径 5px。
- 分支线调色板 12 色，同一条拓扑分支同色。
- 5 万 commit 内流畅：通过视口裁剪只渲染可见行。
- 提交信息遵循：`<type>: <description>`，末尾追加 `Co-Authored-By: Claude <noreply@anthropic.com>`。

---

### Task 1: 定义 Graph 布局类型

**Files:**
- Create: `src/engine/graphTypes.ts`

**Interfaces:**
- Produces: `LayoutNode`, `LayoutRow`, `LanePath`, `GraphLayout` types used by layout engine, renderer, and GraphView.

- [ ] **Step 1: 编写 graphTypes.ts**

```ts
import type { CommitNode, RefPointer } from './types'

export interface LayoutNode {
  commit: CommitNode
  row: number
  lane: number
  parents: LayoutParent[]
}

export interface LayoutParent {
  hash: string
  lane: number // lane assigned to this parent by the layout pass
  isMain: boolean
}

export interface LanePath {
  id: string
  color: string
  segments: PathSegment[]
}

export interface PathSegment {
  x1: number
  y1: number
  x2: number
  y2: number
  curve: boolean // true for merge/fork curves
}

export interface GraphLayout {
  rows: LayoutNode[]
  paths: LanePath[]
  laneCount: number
  rowHeight: number
  laneWidth: number
}

export interface ViewportConfig {
  top: number
  visibleCount: number
  rowHeight: number
}
```

- [ ] **Step 2: 提交**

```bash
git add src/engine/graphTypes.ts
git commit -m "feat(graph): add layout data types

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 实现 Lane 分配纯函数

**Files:**
- Create: `src/engine/layout.ts`
- Create: `tests/vitest/layout.test.ts`

**Interfaces:**
- Consumes: `CommitNode[]`
- Produces: `LayoutNode[]` with `lane` and `parents` populated.

- [ ] **Step 1: 编写 layout.ts**

```ts
import type { CommitNode } from './types'
import type { LayoutNode, LayoutParent } from './graphTypes'

export function computeLayout(commits: CommitNode[]): LayoutNode[] {
  const rows: LayoutNode[] = []
  const laneHeads: (string | null)[] = [] // lane index -> current commit hash at top
  const commitToLane = new Map<string, number>()

  for (let row = 0; row < commits.length; row++) {
    const commit = commits[row]
    let lane = commitToLane.get(commit.id)

    // Try to claim a lane whose head is this commit
    if (lane === undefined) {
      lane = laneHeads.findIndex((head) => head === commit.id)
      if (lane === -1) {
        // New branch: take first free lane or expand
        lane = laneHeads.indexOf(null)
        if (lane === -1) {
          lane = laneHeads.length
          laneHeads.push(null)
        }
      }
    }

    // Mark this lane as occupied by this commit (will be released by parents)
    laneHeads[lane] = commit.id
    commitToLane.set(commit.id, lane)

    // Assign parents to lanes
    const parents: LayoutParent[] = commit.parents.map((parentHash, idx) => {
      const isMain = idx === 0
      let parentLane: number

      if (isMain) {
        parentLane = lane
      } else {
        // Secondary parent: try existing lane head, else new/free lane
        parentLane = laneHeads.findIndex((head) => head === parentHash)
        if (parentLane === -1) {
          parentLane = laneHeads.indexOf(null)
          if (parentLane === -1) {
            parentLane = laneHeads.length
            laneHeads.push(null)
          }
        }
      }

      // Record that this parent continues at parentLane
      if (!commitToLane.has(parentHash)) {
        laneHeads[parentLane] = parentHash
      }

      return { hash: parentHash, lane: parentLane, isMain }
    })

    rows.push({ commit, row, lane, parents })

    // Release this commit's lane for the next iteration unless a parent continues here
    const continuesHere = parents.some((p) => p.lane === lane)
    if (!continuesHere) {
      laneHeads[lane] = null
    }
  }

  return rows
}
```

- [ ] **Step 2: 编写测试**

`tests/vitest/layout.test.ts`:
```ts
import { describe, it, expect } from 'vitest'
import { computeLayout } from '../../src/engine/layout'
import type { CommitNode } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

describe('computeLayout', () => {
  it('lays out linear history on single lane', () => {
    const commits = [node('c3', ['c2']), node('c2', ['c1']), node('c1', [])]
    const layout = computeLayout(commits)
    expect(layout.every((n) => n.lane === 0)).toBe(true)
    expect(layout[0].parents[0].lane).toBe(0)
  })

  it('puts branch on new lane', () => {
    // c3 -> c1, c2 -> c1
    const commits = [node('c3', ['c1']), node('c2', ['c1']), node('c1', [])]
    const layout = computeLayout(commits)
    expect(layout[0].lane).not.toBe(layout[1].lane)
    expect(layout[2].lane).toBe(0)
  })

  it('assigns secondary parent to own lane on merge', () => {
    // c4 merges c3 and c2; both point to c1
    const commits = [node('c4', ['c3', 'c2']), node('c3', ['c1']), node('c2', ['c1']), node('c1', [])]
    const layout = computeLayout(commits)
    const merge = layout[0]
    expect(merge.parents[0].lane).toBe(merge.lane)
    expect(merge.parents[1].lane).not.toBe(merge.lane)
  })
})
```

- [ ] **Step 3: 配置 vitest**

`vitest.config.ts`:
```ts
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import path from 'node:path'

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  test: {
    environment: 'node',
    globals: true,
  },
})
```

- [ ] **Step 4: 运行测试**

Run: `pnpm test`

Expected: 3 tests pass.

- [ ] **Step 5: 提交**

```bash
git add src/engine/layout.ts tests/vitest/layout.test.ts vitest.config.ts
git commit -m "feat(graph): implement lane layout algorithm with tests

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 生成分支线路径

**Files:**
- Create: `src/engine/paths.ts`
- Create: `tests/vitest/paths.test.ts`

**Interfaces:**
- Consumes: `LayoutNode[]`
- Produces: `LanePath[]` (segments between commits in same lane, plus curves for merges/forks).

- [ ] **Step 1: 编写 paths.ts**

```ts
import type { LayoutNode, LanePath, PathSegment } from './graphTypes'

const PALETTE = [
  '#4fc3f7', '#81c784', '#ffb74d', '#ba68c8',
  '#e57373', '#64b5f6', '#fff176', '#4db6ac',
  '#ff8a65', '#90a4ae', '#a1887f', '#7986cb',
]

export function computePaths(rows: LayoutNode[], rowHeight: number, laneWidth: number): LanePath[] {
  const paths = new Map<string, LanePath>()

  function getPath(id: string): LanePath {
    if (!paths.has(id)) {
      paths.set(id, {
        id,
        color: PALETTE[paths.size % PALETTE.length],
        segments: [],
      })
    }
    return paths.get(id)!
  }

  for (const node of rows) {
    const x = node.lane * laneWidth + laneWidth / 2
    const y = node.row * rowHeight + rowHeight / 2

    for (const parent of node.parents) {
      const parentNode = rows.find((r) => r.commit.id === parent.hash)
      if (!parentNode) continue

      const px = parent.lane * laneWidth + laneWidth / 2
      const py = parentNode.row * rowHeight + rowHeight / 2
      const sameLane = parent.lane === node.lane
      const pathId = `${parent.lane}-${node.lane}`
      const path = getPath(pathId)

      const segment: PathSegment = {
        x1: x,
        y1: y,
        x2: px,
        y2: py,
        curve: !sameLane,
      }
      path.segments.push(segment)
    }
  }

  return Array.from(paths.values())
}
```

- [ ] **Step 2: 编写测试**

`tests/vitest/paths.test.ts`:
```ts
import { describe, it, expect } from 'vitest'
import { computeLayout } from '../../src/engine/layout'
import { computePaths } from '../../src/engine/paths'
import type { CommitNode } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

describe('computePaths', () => {
  it('creates one segment per parent link', () => {
    const commits = [node('c2', ['c1']), node('c1', [])]
    const rows = computeLayout(commits)
    const paths = computePaths(rows, 28, 20)
    expect(paths.length).toBe(1)
    expect(paths[0].segments.length).toBe(1)
  })

  it('uses different paths for merge lanes', () => {
    const commits = [node('c4', ['c3', 'c2']), node('c3', ['c1']), node('c2', ['c1']), node('c1', [])]
    const rows = computeLayout(commits)
    const paths = computePaths(rows, 28, 20)
    const mergePath = paths.find((p) => p.segments.some((s) => s.curve))
    expect(mergePath).toBeDefined()
  })
})
```

- [ ] **Step 3: 运行测试**

Run: `pnpm test`

Expected: all pass.

- [ ] **Step 4: 提交**

```bash
git add src/engine/paths.ts tests/vitest/paths.test.ts
git commit -m "feat(graph): compute branch line paths with palette

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 渲染器接口与 SVG 实现

**Files:**
- Create: `src/engine/renderer.ts`
- Create: `src/engine/svgRenderer.ts`

**Interfaces:**
- Consumes: `GraphLayout`, `RefPointer[]`
- Produces: React element tree (SVG).

- [ ] **Step 1: 定义 renderer.ts 接口**

```ts
import type { ReactNode } from 'react'
import type { GraphLayout, RefPointer } from './graphTypes'

export interface GraphRenderer {
  render(layout: GraphLayout, refs: RefPointer[], selectedId: string | null): ReactNode
}
```

- [ ] **Step 2: 实现 svgRenderer.ts**

```ts
import type { ReactNode } from 'react'
import type { GraphLayout, LanePath, LayoutNode, RefPointer } from './graphTypes'
import type { GraphRenderer } from './renderer'

export function createSvgRenderer(): GraphRenderer {
  return {
    render(layout, refs, selectedId): ReactNode {
      const width = Math.max(1, layout.laneCount * layout.laneWidth + 24)
      const height = layout.rows.length * layout.rowHeight
      const refByHash = groupRefs(refs)

      return (
        <svg width={width} height={height} className="graph-svg">
          {layout.paths.map((path) => renderPath(path, layout.rowHeight))}
          {layout.rows.map((node) => renderNode(node, layout, refByHash[node.commit.id] ?? [], selectedId === node.commit.id))}
        </svg>
      )
    },
  }
}

function groupRefs(refs: RefPointer[]): Record<string, RefPointer[]> {
  const map: Record<string, RefPointer[]> = {}
  for (const ref of refs) {
    ;(map[ref.hash] ??= []).push(ref)
  }
  return map
}

function renderPath(path: LanePath, rowHeight: number): ReactNode {
  let d = ''
  for (const s of path.segments) {
    if (s.curve) {
      const midY = (s.y1 + s.y2) / 2
      d += `M ${s.x1} ${s.y1} C ${s.x1} ${midY}, ${s.x2} ${midY}, ${s.x2} ${s.y2} `
    } else {
      d += `M ${s.x1} ${s.y1} L ${s.x2} ${s.y2} `
    }
  }
  return <path key={path.id} d={d} stroke={path.color} fill="none" strokeWidth={2} />
}

function renderNode(node: LayoutNode, layout: GraphLayout, refs: RefPointer[], selected: boolean): ReactNode {
  const cx = node.lane * layout.laneWidth + layout.laneWidth / 2
  const cy = node.row * layout.rowHeight + layout.rowHeight / 2
  const r = 5

  return (
    <g key={node.commit.id} className={selected ? 'commit-node selected' : 'commit-node'} data-id={node.commit.id}>
      {selected && <rect x={cx - 10} y={node.row * layout.rowHeight + 2} width={200} height={layout.rowHeight - 4} rx={4} className="selection-bg" />}
      <circle cx={cx} cy={cy} r={r} stroke={selected ? '#fff' : '#888'} strokeWidth={2} fill={selected ? '#fff' : '#222'} />
      {refs.map((ref, i) => renderRef(ref, cx + 12 + i * 4, cy))}
    </g>
  )
}

function renderRef(ref: RefPointer, x: number, y: number): ReactNode {
  const color = ref.kind === 'head' ? '#fff' : ref.kind === 'remote' ? '#aaa' : '#4fc3f7'
  return (
    <text key={ref.name} x={x} y={y + 3} fill={color} fontSize={10} className="ref-label">
      {ref.name.replace('refs/heads/', '').replace('refs/remotes/', '')}
    </text>
  )
}
```

- [ ] **Step 3: 提交**

```bash
git add src/engine/renderer.ts src/engine/svgRenderer.ts
git commit -m "feat(graph): add renderer interface and SVG implementation

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: GraphView 组件 + 虚拟滚动

**Files:**
- Create: `src/components/GraphView.tsx`
- Create: `src/components/CommitRow.tsx`
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `CommitNode[]`, `RefPointer[]`, callback `onSelect(id)`.
- Produces: scrollable graph view.

- [ ] **Step 1: 编写 GraphView.tsx**

```tsx
import { useMemo, useRef, useState, useCallback } from 'react'
import type { CommitNode, RefPointer } from '../engine/types'
import { computeLayout } from '../engine/layout'
import { computePaths } from '../engine/paths'
import { createSvgRenderer } from '../engine/svgRenderer'

const ROW_HEIGHT = 28
const LANE_WIDTH = 20
const OVERSCAN = 5

interface GraphViewProps {
  commits: CommitNode[]
  refs: RefPointer[]
  selectedId: string | null
  onSelect: (id: string) => void
}

const renderer = createSvgRenderer()

export function GraphView({ commits, refs, selectedId, onSelect }: GraphViewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(600)

  const layout = useMemo(() => {
    const rows = computeLayout(commits)
    const paths = computePaths(rows, ROW_HEIGHT, LANE_WIDTH)
    return {
      rows,
      paths,
      laneCount: Math.max(1, ...rows.map((r) => r.lane)) + 1,
      rowHeight: ROW_HEIGHT,
      laneWidth: LANE_WIDTH,
    }
  }, [commits])

  const startIndex = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN)
  const endIndex = Math.min(
    layout.rows.length - 1,
    Math.floor((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN,
  )
  const visibleRows = layout.rows.slice(startIndex, endIndex + 1)

  const visibleLayout = useMemo(() => {
    const paths = computePaths(visibleRows, ROW_HEIGHT, LANE_WIDTH)
    return {
      ...layout,
      rows: visibleRows,
      paths,
    }
  }, [visibleRows, layout])

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    setScrollTop(e.currentTarget.scrollTop)
    setViewportHeight(e.currentTarget.clientHeight)
  }, [])

  const handleClick = useCallback((e: React.MouseEvent) => {
    const target = e.target as HTMLElement
    const id = target.closest('[data-id]')?.getAttribute('data-id')
    if (id) onSelect(id)
  }, [onSelect])

  const totalHeight = layout.rows.length * ROW_HEIGHT
  const svg = renderer.render(visibleLayout, refs, selectedId)

  return (
    <div
      ref={containerRef}
      className="graph-view"
      onScroll={handleScroll}
      onClick={handleClick}
      style={{ overflow: 'auto', height: '100%' }}
    >
      <div style={{ position: 'relative', height: totalHeight, minWidth: layout.laneCount * LANE_WIDTH + 300 }}>
        <div style={{ position: 'absolute', top: startIndex * ROW_HEIGHT, left: 0 }}>
          {svg}
        </div>
      </div>
    </div>
  )
}
```

- [ ] **Step 2: 最简 App 连接 mock 数据**

`src/App.tsx`:
```tsx
import { useState } from 'react'
import { GraphView } from './components/GraphView'
import type { CommitNode, RefPointer } from './engine/types'

const mockCommits: CommitNode[] = [
  { id: 'c4', parents: ['c3', 'c2'], author: 'a', time: 4, message: 'Merge feature' },
  { id: 'c3', parents: ['c1'], author: 'a', time: 3, message: 'Feature work' },
  { id: 'c2', parents: ['c1'], author: 'a', time: 2, message: 'Bug fix' },
  { id: 'c1', parents: [], author: 'a', time: 1, message: 'Initial commit' },
]

const mockRefs: RefPointer[] = [
  { name: 'main', hash: 'c4', kind: 'branch' },
  { name: 'feature', hash: 'c3', kind: 'branch' },
]

function App() {
  const [selected, setSelected] = useState<string | null>(null)
  return (
    <div style={{ height: '100vh' }}>
      <GraphView commits={mockCommits} refs={mockRefs} selectedId={selected} onSelect={setSelected} />
    </div>
  )
}

export default App
```

- [ ] **Step 3: 添加基础 CSS**

`src/styles.css`:
```css
:root {
  --bg: #1e1e1e;
  --fg: #d4d4d4;
  --selected-bg: rgba(255, 255, 255, 0.1);
}

body {
  margin: 0;
  background: var(--bg);
  color: var(--fg);
  font-family: system-ui, -apple-system, sans-serif;
  font-size: 12px;
}

.graph-view {
  background: var(--bg);
}

.commit-node {
  cursor: pointer;
}

.selection-bg {
  fill: var(--selected-bg);
}

.ref-label {
  font-family: monospace;
}
```

- [ ] **Step 4: 运行验证**

Run: `pnpm tauri dev`

Expected: 窗口显示 4 个 commit 的 graph，分支线有色，点击 commit 可选中。

- [ ] **Step 5: 提交**

```bash
git add src/components/GraphView.tsx src/components/CommitRow.tsx src/App.tsx src/styles.css
git commit -m "feat(graph): add GraphView with viewport clipping and selection

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: 提交信息行 + 指针标签 + 键盘导航

**Files:**
- Create: `src/components/CommitRow.tsx`
- Modify: `src/components/GraphView.tsx`
- Modify: `src/engine/svgRenderer.ts`

**Interfaces:**
- Produces: each commit row shows message, author, time, and ref pills.

- [ ] **Step 1: 修改 svgRenderer 只画点和线**

让 `svgRenderer` 只渲染 SVG 的点和线；文本行放在 HTML 层（性能更好，可复制）。

Update `svgRenderer.ts` `renderNode` to remove text rendering.

- [ ] **Step 2: 编写 CommitRow.tsx**

```tsx
import type { LayoutNode, RefPointer } from '../engine/graphTypes'

interface CommitRowProps {
  node: LayoutNode
  refs: RefPointer[]
  selected: boolean
  layoutWidth: number
}

export function CommitRow({ node, refs, selected, layoutWidth }: CommitRowProps) {
  const date = new Date(node.commit.time * 1000).toLocaleString()
  return (
    <div
      className={selected ? 'commit-row selected' : 'commit-row'}
      style={{ height: 28, display: 'flex', alignItems: 'center', paddingLeft: layoutWidth + 8 }}
    >
      <span className="commit-message" style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {node.commit.message}
      </span>
      <span className="commit-meta" style={{ color: '#888', marginLeft: 8, whiteSpace: 'nowrap' }}>
        {node.commit.author} • {date}
      </span>
      <span style={{ marginLeft: 8, display: 'flex', gap: 4 }}>
        {refs.map((ref) => (
          <span key={ref.name} className={`ref-pill ${ref.kind}`}>
            {ref.name.replace('refs/heads/', '').replace('refs/remotes/', '')}
          </span>
        ))}
      </span>
    </div>
  )
}
```

- [ ] **Step 3: 更新 GraphView 增加 HTML 行与键盘导航**

Modify GraphView to render visible `CommitRow` components alongside SVG.

Add keyboard handler in GraphView:
```tsx
useEffect(() => {
  const handler = (e: KeyboardEvent) => {
    if (!selectedId) return
    const idx = commits.findIndex((c) => c.id === selectedId)
    if (e.key === 'ArrowUp' && idx > 0) onSelect(commits[idx - 1].id)
    if (e.key === 'ArrowDown' && idx < commits.length - 1) onSelect(commits[idx + 1].id)
  }
  window.addEventListener('keydown', handler)
  return () => window.removeEventListener('keydown', handler)
}, [commits, selectedId, onSelect])
```

- [ ] **Step 4: 更新 CSS 增加行样式与 ref pills**

```css
.commit-row {
  display: flex;
  align-items: center;
  cursor: pointer;
}

.commit-row:hover,
.commit-row.selected {
  background: var(--selected-bg);
}

.ref-pill {
  padding: 1px 6px;
  border-radius: 10px;
  font-size: 10px;
  color: #fff;
  background: #4fc3f7;
}

.ref-pill.remote {
  background: transparent;
  border: 1px solid #aaa;
  color: #aaa;
}
```

- [ ] **Step 5: 运行验证**

Run: `pnpm tauri dev`

Expected: 每行显示消息、作者时间、分支标签；上下键可导航。

- [ ] **Step 6: 提交**

```bash
git add src/components/GraphView.tsx src/components/CommitRow.tsx src/styles.css
git commit -m "feat(graph): add commit row text, ref pills, and keyboard navigation

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- 自研布局算法 ✅ Task 2
- lane 分配/合并线/分叉 ✅ Task 2, 3
- 分支颜色/指针标记 ✅ Task 3, 6
- SVG 渲染器可替换 ✅ Task 4
- 视口裁剪 ✅ Task 5
- 交互（点击、键盘导航）✅ Task 5, 6

**Placeholder scan:** 无占位符。

**Type consistency:** `CommitNode`/`RefPointer` 与 Plan 1 类型一致；`LayoutNode` 在本计划中首次定义。

**未覆盖（留给 Plan 3）：** 真实仓库数据接入、分支/worktree 管理面板、staging/commit/diff、刷新机制、右键菜单、写操作命令。
