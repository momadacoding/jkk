import { useCallback, useRef } from 'react'
import {
  useDefaultLayout,
  useGroupRef,
  type Layout,
  type LayoutChangedMeta,
  type LayoutStorage,
} from 'react-resizable-panels'
import {
  SECTION_MEMORY_KEY,
  layoutToRestore,
  type PanelGroupId,
  type PanelLayout,
  parseSectionMemory,
  sanitizePanelLayout,
  sectionOpen,
  withSectionOpen,
  type SidebarSectionKey,
} from '@/engine/layoutMemory'

/**
 * 布局记忆的读写。
 *
 * 存在 WebView 的 `localStorage` 而不是 `app_config_dir` 下的 JSON：
 * - 挂载时同步读到，首帧就是记住的布局，不会先按默认画一帧再跳；
 * - 布局是纯展示偏好，丢了只是回到默认，不值得为它加 Tauri 命令；
 * - dev 实例和安装版的 origin 不同，各记各的，不会互相改写。
 *
 * 所有访问都包 try：隐私模式、存储被禁用或配额满时 `localStorage` 会抛，
 * 布局记不住可以接受，侧边栏因此渲染不出来不行。
 */
function storage(): Storage | null {
  try {
    return globalThis.localStorage ?? null
  } catch {
    return null
  }
}

function readRaw(key: string): string | null {
  try {
    return storage()?.getItem(key) ?? null
  } catch {
    return null
  }
}

function writeRaw(key: string, value: string): void {
  try {
    storage()?.setItem(key, value)
  } catch {
    // 写不进去就算了，下次启动回到默认布局
  }
}

/** 读一个分区的展开状态；没记过就用 `fallback`。 */
export function loadSectionOpen(key: SidebarSectionKey, fallback: boolean): boolean {
  return sectionOpen(parseSectionMemory(readRaw(SECTION_MEMORY_KEY)), key, fallback)
}

/** 记住一个分区的展开状态。每次都重读再合并，多个仓库标签各自写不会互相覆盖。 */
export function saveSectionOpen(key: SidebarSectionKey, open: boolean): void {
  const next = withSectionOpen(parseSectionMemory(readRaw(SECTION_MEMORY_KEY)), key, open)
  writeRaw(SECTION_MEMORY_KEY, JSON.stringify(next))
}

/** 给 `useDefaultLayout` 用的存储：读出来的存档先校验，坏的当作没有。 */
const panelLayoutStorage: LayoutStorage = {
  getItem: (key) => sanitizePanelLayout(readRaw(key)),
  setItem: (key, value) => writeRaw(key, value),
}

/**
 * 读 `useDefaultLayout` 存下的布局。键是库拼的 `react-resizable-panels:<id>`
 * （没传 `panelIds` 时的格式，4.12.2 实测）；升级库后若格式变了，这里读不到只会退化成不补布局。
 */
function readPanelLayout(id: PanelGroupId): PanelLayout | undefined {
  const raw = sanitizePanelLayout(readRaw(`react-resizable-panels:${id}`))
  return raw === null ? undefined : (JSON.parse(raw) as PanelLayout)
}

/**
 * 记住一个 `<Group>` 的分割比例，返回值直接展开到 Group 上：`<Group {...usePanelLayout(id)}>`。
 *
 * 只存用户亲手拖出来的布局——窗口缩放、挂载时库自己算出来的布局不该覆盖记忆。
 * Group 里的每个 `<Panel>` 都要给固定的 `id`，存档按面板 id 对应；
 * 不要把 `id` 也传给 `<Group>`，原因见 `PANEL_GROUP_IDS`。
 *
 * 隐藏状态下挂载的 Group 要补一步：App 把后台标签挂着、用 display:none 藏起来，
 * 这时 Group 量到的尺寸是 0，库会把 `defaultLayout` 搁置，等第一次真正有尺寸时
 * 改用各 Panel 的 `defaultSize` 重算——记住的比例就被跳过了。所以每次 Group 挂载后，
 * 在它第一次有尺寸的那次布局变化里，用 `setLayout` 把存档补上去。
 * 放在这里而不是让调用方「先别挂载」，是因为嵌套的 Group 也会在标签隐藏时重新挂载
 * （比如后台刷新让提交详情的 diff 重新加载），每一处都得兜住。
 */
export function usePanelLayout(id: PanelGroupId) {
  const { defaultLayout, onLayoutChanged } = useDefaultLayout({
    id,
    storage: panelLayoutStorage,
    onlySaveAfterUserInteractions: true,
  })
  const groupRef = useGroupRef()
  const elementRef = useRef<HTMLDivElement | null>(null)
  /** 这次挂载之后还没见过有尺寸的布局变化。每次 Group 重新挂载都重置。 */
  const pendingRef = useRef(true)

  // 回调 ref 的身份必须稳定：只在 Group 挂载 / 卸载时被调用，正好用来重置 pending
  const attachElement = useCallback((el: HTMLDivElement | null) => {
    elementRef.current = el
    if (el) pendingRef.current = true
  }, [])

  const handleLayoutChanged = useCallback(
    (layout: Layout, meta: LayoutChangedMeta) => {
      onLayoutChanged(layout, meta)
      if (!pendingRef.current) return
      const el = elementRef.current
      if (!el || el.offsetWidth === 0 || el.offsetHeight === 0) return
      pendingRef.current = false
      // 用户亲手拖的就是要的布局，不能拿存档去盖
      if (meta.isUserInteraction) return
      // 补的时候现读存档，不用渲染时拿到的 defaultLayout：隐藏的标签不重渲染，
      // 期间用户在别的标签里又拖过，那份就旧了
      const target = layoutToRestore(readPanelLayout(id), layout)
      // setLayout 引起的下一次布局变化 pending 已经是 false，不会再进来，不会循环
      if (target) groupRef.current?.setLayout(target)
    },
    [onLayoutChanged, groupRef, id],
  )

  return { defaultLayout, onLayoutChanged: handleLayoutChanged, groupRef, elementRef: attachElement }
}
