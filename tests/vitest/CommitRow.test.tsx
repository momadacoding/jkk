// @vitest-environment jsdom
//
// 混合基线那一行：标注与 svn/base 标签（计划 Task 12）。jsdom 不渲染布局，这里只看内容。
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { CommitRow } from '@/components/CommitRow'
import { baselineMap, remapRefs, spliceHistory } from '@/engine/svnGraph'
import type { LayoutNode, RefPointer } from '@/engine/graphTypes'
import type { CommitNode } from '@/engine/types'

const mixed: CommitNode = {
  id: 'abcdef1234567890',
  parents: ['B'],
  author: 'me',
  time: 1_700_000_000,
  message: 'svn: r10（另有 3 个文件在更新的 revision）',
  baselineFloor: 10,
  baselineNewer: 3,
}
const single: CommitNode = {
  id: 'B',
  parents: [],
  author: 'me',
  time: 1_699_000_000,
  message: 'svn: r10',
  baselineRev: 10,
}
const baseRef: RefPointer = {
  name: 'refs/heads/svn/base',
  hash: mixed.id,
  kind: 'branch',
  ahead: null,
  behind: null,
  goneUpstream: null,
}

function row(commit: CommitNode, refs: RefPointer[]) {
  render(
    <CommitRow
      node={{ commit } as LayoutNode}
      refs={refs}
      prByBranch={new Map()}
      selected={false}
      layoutWidth={0}
      laneColor="#fff"
      changeCount={0}
      isHead={false}
    />,
  )
}

describe('CommitRow：混合基线', () => {
  it('标注 floor 与文件数，svn/base 标签留在这一行', () => {
    const r10 = { rev: 10, author: 'x', date: '2026-10-11T00:00:00Z', message: 'r10' }
    const commits = spliceHistory([r10], [mixed, single], [mixed.id])
    const refs = remapRefs([baseRef], baselineMap([mixed, single]))
    row(commits.find((c) => c.id === mixed.id)!, refs.filter((r) => r.hash === mixed.id))

    expect(screen.getByText('svn/base：r10 + 3 个文件在更新的 revision')).toBeTruthy()
    expect(screen.getByText('svn/base')).toBeTruthy()
    // 标题是 format_message 拼的同一句话，一行里不重复两遍
    expect(screen.queryByText(mixed.message)).toBeNull()
  })

  it('普通提交没有这个标注', () => {
    const { baselineFloor: _f, baselineNewer: _n, ...plain } = mixed
    row({ ...plain, message: '修跳跃', vcs: 'git' }, [])
    expect(screen.getByText('修跳跃')).toBeTruthy()
    expect(screen.queryByText(/svn\/base：/)).toBeNull()
  })
})
