// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useSvnConflictsDialog, type SvnConflictsRequest } from '@/components/SvnConflictsDialog'
import type { SvnConflict } from '@/engine/svnConflicts'

const X: SvnConflict = {
  path: 'Art/UI_Main.prefab',
  text: true,
  props: false,
  tree: false,
  dir: false,
  gitManaged: false,
  resolvable: true,
  mergeable: true,
}
const TREE: SvnConflict = { ...X, path: 'Art/Old', text: false, tree: true, resolvable: false, mergeable: false }

function open(over: Partial<SvnConflictsRequest> = {}) {
  let left: SvnConflict[] = [X, TREE]
  const resolve = vi.fn(async (file: string) => {
    left = left.filter((c) => c.path !== file)
  })
  const load = vi.fn(async () => left)
  const opened = vi.fn(async () => {})
  let show!: (r: SvnConflictsRequest) => Promise<void>
  function Harness() {
    const { showSvnConflicts, dialog } = useSvnConflictsDialog()
    show = showSvnConflicts
    return <>{dialog}</>
  }
  render(<Harness />)
  let closed!: Promise<void>
  act(() => {
    closed = show({
      initial: left,
      load,
      resolve,
      merge: () => ({ label: 'UnityYAMLMerge', open: opened }),
      ...over,
    })
  })
  return { resolve, load, opened, closed: () => closed }
}

const btn = (name: string) => screen.getByRole('button', { name })

describe('SvnConflictsDialog', () => {
  it('能解决的一行四个按钮；树冲突单列、没有按钮；焦点在「关闭」上', async () => {
    open()
    expect(await screen.findByText('Art/UI_Main.prefab')).toBeTruthy()
    for (const name of ['用我的', '用服务器的', '用 UnityYAMLMerge 合并', '标记已解决']) {
      expect(btn(`Art/UI_Main.prefab：${name}`)).toBeTruthy()
    }
    expect(screen.getByText('Art/Old')).toBeTruthy()
    expect(screen.getByText('树冲突')).toBeTruthy()
    expect(screen.queryByRole('button', { name: /^Art\/Old：/ })).toBeNull()
    expect(document.activeElement).toBe(btn('关闭'))
  })

  it('「用服务器的」先在行里确认；确认之后才解决，解决完重读列表', async () => {
    const { resolve, load } = open()
    fireEvent.click(btn('Art/UI_Main.prefab：用服务器的'))
    expect(resolve).not.toHaveBeenCalled()
    expect(screen.getByText(/本地的改动丢掉/)).toBeTruthy()

    fireEvent.click(btn('确定用服务器的'))

    await waitFor(() => expect(resolve).toHaveBeenCalledWith('Art/UI_Main.prefab', 'theirs'))
    await waitFor(() => expect(load).toHaveBeenCalled())
    await waitFor(() => expect(screen.queryByText('Art/UI_Main.prefab')).toBeNull())
  })

  it('确认时点「不改了」：什么都不做', async () => {
    const { resolve } = open()
    fireEvent.click(btn('Art/UI_Main.prefab：用我的'))
    fireEvent.click(btn('不改了'))
    expect(screen.queryByText(/服务器这次的改动丢掉/)).toBeNull()
    expect(resolve).not.toHaveBeenCalled()
  })

  it('解决失败：错误挂在那一行下面，行还在', async () => {
    open({
      resolve: async () => {
        throw { code: 'svn_conflict_markers_left', message: '还有冲突标记' }
      },
    })
    fireEvent.click(btn('Art/UI_Main.prefab：标记已解决'))
    expect(await screen.findByText(/还有冲突标记/)).toBeTruthy()
    expect(screen.getByText('Art/UI_Main.prefab')).toBeTruthy()
  })

  it('外部合并：打开工具，不重读、不标记已解决', async () => {
    const { opened, resolve, load } = open()
    fireEvent.click(btn('Art/UI_Main.prefab：用 UnityYAMLMerge 合并'))
    await waitFor(() => expect(opened).toHaveBeenCalled())
    expect(resolve).not.toHaveBeenCalled()
    expect(load).not.toHaveBeenCalled()
  })

  it('没配合并工具：不给「合并」和「标记已解决」', async () => {
    open({ merge: () => null })
    await screen.findByText('Art/UI_Main.prefab')
    expect(screen.queryByRole('button', { name: /合并$/ })).toBeNull()
    expect(screen.queryByRole('button', { name: /标记已解决$/ })).toBeNull()
    expect(btn('Art/UI_Main.prefab：用我的')).toBeTruthy()
  })

  it('点「关闭」：框消失，Promise 结束', async () => {
    const { closed } = open()
    fireEvent.click(btn('关闭'))
    await expect(closed()).resolves.toBeUndefined()
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('一个冲突都不剩时显示「冲突都解决了」', async () => {
    open({ initial: [X], load: async () => [] })
    fireEvent.click(btn('Art/UI_Main.prefab：用我的'))
    fireEvent.click(btn('确定用我的'))
    expect(await screen.findByText('冲突都解决了。')).toBeTruthy()
  })
})
