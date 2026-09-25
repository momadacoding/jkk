import type { RefPointer } from './types'

const PREFIXES = ['refs/heads/', 'refs/remotes/', 'refs/tags/']
const REMOTE_PREFIX = 'refs/remotes/'

/** 去掉 refs/heads/、refs/remotes/、refs/tags/ 前缀，标签上只留下人看的部分。 */
export function shortRefName(name: string): string {
  const prefix = PREFIXES.find((p) => name.startsWith(p))
  return prefix ? name.slice(prefix.length) : name
}

/** graph 行上的一个标签。 */
export interface RefPill {
  /** React key，取标签所代表的 ref 全名。 */
  key: string
  kind: RefPointer['kind']
  label: string
  /**
   * 标签代表的 ref。合并出来的标签是**本地分支**——PR 编号与 merged-ahead
   * 判断都要拿本地分支的名字和 tip，见 `pullRequestForRef` / `prBadge`。
   */
  ref: RefPointer
  /** 并进这个本地分支标签的远程分支短名（如 `origin/main`）；没有则为空数组。 */
  remotes: string[]
}

/**
 * 远程分支短名里去掉 remote 名之后的分支名；拆不出来返回 null。
 *
 * 按**第一个** `/` 拆：remote 名本身带斜杠（合法但罕见）时会拆错，后果只是
 * 一个远程标签被并进同名本地分支、或该并的没并——两个 ref 本来就在同一个
 * 提交上，合并与否都不会误导「分支在哪」，悬停提示里也写着完整远程名。
 */
function remoteBranchName(ref: RefPointer): string | null {
  if (ref.kind !== 'remote' || !ref.name.startsWith(REMOTE_PREFIX)) return null
  const short = ref.name.slice(REMOTE_PREFIX.length)
  const slash = short.indexOf('/')
  if (slash <= 0 || slash === short.length - 1) return null
  return short.slice(slash + 1)
}

/**
 * 把一行上的 refs 变成标签列表：本地分支与**同一行上**的同名远程分支合成一个标签。
 *
 * 同一行即同一个提交，所以合并后的标签只在「本地与远程指向同一提交」时出现；
 * 两边分叉时它们落在不同的行上，照旧各画各的，分叉这件事不会被藏掉。
 *
 * 顺序沿用输入顺序，被并掉的远程分支从原位置移除。
 */
export function buildRefPills(refs: RefPointer[]): RefPill[] {
  const localNames = new Set(
    refs.filter((r) => r.kind === 'branch').map((r) => shortRefName(r.name)),
  )

  // 本地分支短名 → 并进来的远程分支短名
  const merged = new Map<string, string[]>()
  const absorbed = new Set<string>()
  for (const r of refs) {
    const branch = remoteBranchName(r)
    if (branch === null || !localNames.has(branch)) continue
    const list = merged.get(branch) ?? []
    list.push(shortRefName(r.name))
    merged.set(branch, list)
    absorbed.add(r.name)
  }

  return refs
    .filter((r) => !absorbed.has(r.name))
    .map((r) => {
      const label = shortRefName(r.name)
      return {
        key: r.name,
        kind: r.kind,
        label,
        ref: r,
        remotes: r.kind === 'branch' ? merged.get(label) ?? [] : [],
      }
    })
}
