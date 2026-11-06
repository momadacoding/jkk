import type { LayoutStorage } from 'react-resizable-panels'
import {
  SECTION_MEMORY_KEY,
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
export const panelLayoutStorage: LayoutStorage = {
  getItem: (key) => sanitizePanelLayout(readRaw(key)),
  setItem: (key, value) => writeRaw(key, value),
}
