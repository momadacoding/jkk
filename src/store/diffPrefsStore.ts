import { useSyncExternalStore } from 'react'
import { DIFF_PREFS_KEY, backendDiffOptions, diffOptionsKey, parseDiffPrefs, type DiffPrefs } from '@/engine/diffPrefs'

/**
 * diff 面板开关的全局存储。所有标签在同一个页面里，模块级的一份就够了，换哪个开关所有 diff 视图一起变。
 *
 * 存 `localStorage`，与布局记忆同一取舍（`layoutStore.ts`）：首帧就能同步读到，丢了只是回到默认。
 * 所有访问都包 try：存储被禁用时 `localStorage` 会抛，开关记不住可以接受，diff 面板因此打不开不行。
 */
function readStored(): string | null {
  try {
    return globalThis.localStorage?.getItem(DIFF_PREFS_KEY) ?? null
  } catch {
    return null
  }
}

function writeStored(prefs: DiffPrefs): void {
  try {
    globalThis.localStorage?.setItem(DIFF_PREFS_KEY, JSON.stringify(prefs))
  } catch {
    // 写不进去就算了，下次启动回到默认
  }
}

let current: DiffPrefs = parseDiffPrefs(readStored())
const listeners = new Set<() => void>()

/** 当前的开关。store 在发起 diff 读取的那一刻调它，取当时的值。 */
export function getDiffPrefs(): DiffPrefs {
  return current
}

export function setDiffPrefs(patch: Partial<DiffPrefs>): void {
  current = { ...current, ...patch }
  writeStored(current)
  for (const notify of listeners) notify()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

export function useDiffPrefs(): DiffPrefs {
  return useSyncExternalStore(subscribe, getDiffPrefs)
}

function optionsKeySnapshot(): string {
  return diffOptionsKey(backendDiffOptions(current))
}

/**
 * 只订阅要后端重读的那部分（`diffOptionsKey`）。快照是字符串，切换分栏、换行这些画法开关时不变，
 * 用它的组件（每个标签的 `useRepoEffects`）就不跟着重渲染（独立 review 第 4 条）。
 */
export function useDiffOptionsKey(): string {
  return useSyncExternalStore(subscribe, optionsKeySnapshot)
}
