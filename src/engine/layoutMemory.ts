/**
 * 布局记忆的纯逻辑：侧边栏各分区的展开状态、面板分割比例怎么存、怎么读。
 *
 * 只管解析与校验，不碰 `localStorage`——读写在 `src/store/layoutStore.ts`。
 * 存档是程序自己写的，坏了只说明被手改过或写到一半，一律当作没有记忆、
 * 回到默认布局，不能把解析错误顶到渲染路径上（侧边栏渲染不出来比忘了布局糟得多）。
 */

/** 侧边栏里会被记住展开状态的分区。新增分区要在这里登记，避免键名手写拼错。 */
export type SidebarSectionKey =
  | 'branches'
  | 'branches.local'
  | 'branches.remote'
  | 'tags'
  | 'worktrees'
  | 'stashes'

/** 分区展开状态：只记用户点过的分区，没记录的走组件自己的默认值。 */
export type SectionMemory = Partial<Record<SidebarSectionKey, boolean>>

/** 分区展开状态在 `localStorage` 里的键。带版本号，将来改形状时换键即可，不用迁移。 */
export const SECTION_MEMORY_KEY = 'mygitclient.layout.sections.v1'

/**
 * 面板分割比例的存档 id，传给 `useDefaultLayout`，库会拿它拼出存储键。
 * 不要同时传给 `<Group id>`：那个 id 在库的全局注册表里必须唯一，而每个仓库标签都有一套。
 */
export const PANEL_GROUP_IDS = {
  /** 侧边栏 | 主区域 */
  repoMain: 'mygitclient.layout.repo-main.v1',
  /** Commits 视图里的 提交图 / 提交详情 */
  repoCommits: 'mygitclient.layout.repo-commits.v1',
} as const

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** 解析分区展开状态；坏 JSON、形状不对、值不是布尔的条目一律丢掉。 */
export function parseSectionMemory(raw: string | null): SectionMemory {
  if (raw === null) return {}
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return {}
  }
  if (!isPlainObject(parsed)) return {}
  const result: Record<string, boolean> = {}
  for (const [key, value] of Object.entries(parsed)) {
    if (typeof value === 'boolean') result[key] = value
  }
  return result as SectionMemory
}

/** 有记忆用记忆，没有就用组件给的默认值。 */
export function sectionOpen(memory: SectionMemory, key: SidebarSectionKey, fallback: boolean): boolean {
  return memory[key] ?? fallback
}

/** 记下一个分区的展开状态，返回新对象，不改入参。 */
export function withSectionOpen(memory: SectionMemory, key: SidebarSectionKey, open: boolean): SectionMemory {
  return { ...memory, [key]: open }
}

/**
 * 校验面板布局存档，合法时原样返回，否则返回 `null`（等同于没有存档）。
 *
 * 这一层必须挡在 `react-resizable-panels` 的 `useDefaultLayout` 前面：
 * 它在渲染期直接 `JSON.parse` 存档、没有 try，存档一坏整个仓库标签就白屏。
 * 合法形状是「面板 id → 非负有限数」的对象，且至少有一项。
 */
export function sanitizePanelLayout(raw: string | null): string | null {
  if (raw === null) return null
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }
  if (!isPlainObject(parsed)) return null
  const values = Object.values(parsed)
  if (values.length === 0) return null
  const valid = values.every((v) => typeof v === 'number' && Number.isFinite(v) && v >= 0)
  return valid ? raw : null
}
