import { describe, expect, it } from 'vitest'
import { WORKDIR_ID, withWorkdirRow } from '../../src/engine/workdir'
import { computeLayout } from '../../src/engine/layout'
import { computePaths } from '../../src/engine/paths'
import type { CommitNode, RefPointer } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

function ref(name: string, hash: string, kind: RefPointer['kind']): RefPointer {
  return { name, hash, kind }
}

const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
const refs = [
  ref('refs/heads/master', 'm1', 'branch'),
  ref('refs/heads/feature', 'f1', 'branch'),
]

describe('withWorkdirRow', () => {
  it('把伪行插在最前面，父指向 HEAD', () => {
    const out = withWorkdirRow(commits, 'm1')
    expect(out[0].id).toBe(WORKDIR_ID)
    expect(out[0].parents).toEqual(['m1'])
    expect(out.slice(1)).toEqual(commits)
  })

  it('没有 HEAD（空仓库）时不插伪行', () => {
    expect(withWorkdirRow(commits, null)).toEqual(commits)
  })
})

describe('工作区伪行的布局', () => {
  it('伪行取 HEAD 提交的车道', () => {
    const rows = computeLayout(withWorkdirRow(commits, 'f1'), refs)
    const byId = Object.fromEntries(rows.map((r) => [r.commit.id, r]))
    expect(byId[WORKDIR_ID].lane).toBe(byId.f1.lane)
  })

  it('HEAD 在 master 上时伪行仍在 lane 0，不会凭空多出一列', () => {
    // 若让伪行参与装箱，它会退化成只占一行的独立 owner 被丢到 lane 1
    const rows = computeLayout(withWorkdirRow(commits, 'm1'), refs)
    const byId = Object.fromEntries(rows.map((r) => [r.commit.id, r]))
    expect(byId[WORKDIR_ID].lane).toBe(0)
  })

  it('伪行不改变任何真实提交的车道', () => {
    const without = computeLayout(commits, refs)
    const with_ = computeLayout(withWorkdirRow(commits, 'f1'), refs)
    const lanes = (rows: typeof without) =>
      Object.fromEntries(rows.filter((r) => r.commit.id !== WORKDIR_ID).map((r) => [r.commit.id, r.lane]))
    expect(lanes(with_)).toEqual(lanes(without))
  })

  it('HEAD 落在哪个分支都不改变真实提交的车道', () => {
    const onMaster = computeLayout(withWorkdirRow(commits, 'm1'), refs)
    const onFeature = computeLayout(withWorkdirRow(commits, 'f1'), refs)
    const lanes = (rows: typeof onMaster) =>
      Object.fromEntries(rows.filter((r) => r.commit.id !== WORKDIR_ID).map((r) => [r.commit.id, r.lane]))
    expect(lanes(onFeature)).toEqual(lanes(onMaster))
  })
})

describe('虚线标记', () => {
  it('只有伪行连出去的线段是虚线', () => {
    const rows = computeLayout(withWorkdirRow(commits, 'f1'), refs)
    const paths = computePaths(rows, 28, 20)
    const all = paths.flatMap((p) => p.segments)
    expect(all.filter((s) => s.dashed)).toHaveLength(1)
    expect(all.filter((s) => !s.dashed).length).toBeGreaterThan(0)
  })

  it('没有伪行时一条虚线都没有', () => {
    const paths = computePaths(computeLayout(commits, refs), 28, 20)
    expect(paths.flatMap((p) => p.segments).some((s) => s.dashed)).toBe(false)
  })
})
