import type { CommitNode, RefPointer } from './types'

/** 去掉 `refs/tags/` 前缀，拿到侧边栏和 pill 上显示的短名。 */
export function tagShortName(name: string): string {
  return name.replace('refs/tags/', '')
}

/**
 * 把 tag 按「新的在上」排序。
 *
 * 不能直接用 git 给的顺序：`repo.references()` 是字典序，`v0.10.0` 会排在
 * `v0.2.0` 前面——版本号一上两位数，列表看着就是乱的。
 *
 * 也不做语义化版本解析：tag 不一定是版本号（`nightly-2026-08-12`、`rc`、
 * 中文名都可能），解析失败的回退规则比按时间排更难预测。目标提交的时间是
 * 所有 tag 都有的、且和用户心里的「哪个更新」最接近的排序键。
 *
 * 目标提交不在图里的排到最后：它们点了也定位不了，不该占着列表顶部。
 * 时间相同（同一条提交上打了多个 tag，或同秒提交）时按名字倒序，
 * 让 `v1.2` 排在 `v1.10` 上面之类的问题至少是稳定可预期的。
 */
export function sortTags(tags: RefPointer[], commits: CommitNode[]): RefPointer[] {
  const timeOf = new Map<string, number>()
  for (const c of commits) timeOf.set(c.id, c.time)

  return [...tags].sort((a, b) => {
    const ta = timeOf.get(a.hash)
    const tb = timeOf.get(b.hash)
    // 不在图里的沉底；两个都不在则只按名字排，不要让顺序随数组原顺序漂移
    if (ta === undefined && tb !== undefined) return 1
    if (ta !== undefined && tb === undefined) return -1
    if (ta !== undefined && tb !== undefined && ta !== tb) return tb - ta
    return b.name.localeCompare(a.name)
  })
}
