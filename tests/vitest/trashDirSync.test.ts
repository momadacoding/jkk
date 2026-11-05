/**
 * 第 68 条：删除 worktree 的确认框写着回收站目录，那是撤销入口删掉之后用户手工捞回 worktree
 * 的唯一线索。新槽位挪到 `.trash-v2` 时文案漏改过一次（后来已改对）。
 *
 * 这条只是防线：读源码把两边钉在一起，下次再改 `TRASH_DIR`，文案没跟上就会红。
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const ROOT = process.cwd()

function rustConst(name: string): string {
  const src = readFileSync(join(ROOT, 'src-tauri/src/commands/worktree.rs'), 'utf8')
  const m = new RegExp(`const ${name}: &str = "([^"]+)";`).exec(src)
  if (!m) throw new Error(`worktree.rs 里找不到 ${name}`)
  return m[1]
}

describe('删除 worktree 确认框里的回收站目录', () => {
  const dialog = readFileSync(join(ROOT, 'src/components/DeleteWorktreeDialog.tsx'), 'utf8')
  const mentioned = [...dialog.matchAll(/\.worktrees\/(\.trash[\w-]*)/g)].map((m) => m[1])

  it('写出了回收站目录', () => {
    expect(mentioned.length).toBeGreaterThan(0)
  })

  it('写的就是后端新槽位所在的 TRASH_DIR，不是旧版目录', () => {
    const dir = rustConst('TRASH_DIR')
    expect(dir).not.toBe(rustConst('TRASH_DIR_LEGACY'))
    expect(new Set(mentioned)).toEqual(new Set([dir]))
  })
})
