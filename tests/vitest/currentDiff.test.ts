import { describe, expect, it } from 'vitest'
import { currentDiff } from '@/engine/currentDiff'
import type { DiffData } from '@/engine/types'

const workdirDiff: DiffData = {
  files: [{ path: 'Assets/D0000/F001.cs', status: 'modified', binary: false, hunks: [] }],
}

// 2026-10-22 E2E：选中 5 万文件的根提交，diff 要几十秒才回来，这期间提交详情显示的是工作区的改动
describe('currentDiff', () => {
  it('diff 是按当前视图要的 scope 读的：照常显示', () => {
    expect(currentDiff(workdirDiff, 'workdir', 'workdir')).toBe(workdirDiff)
  })

  it('换了选中、新 diff 还没回来：不能把上一份当成新选中的内容', () => {
    expect(currentDiff(workdirDiff, 'workdir', 'commit:24ea42f')).toBeNull()
  })

  it('从提交详情切回本地改动、工作区 diff 还没回来：同样不显示', () => {
    expect(currentDiff(workdirDiff, 'commit:24ea42f', 'workdir')).toBeNull()
  })

  it('还没读过任何 diff', () => {
    expect(currentDiff({ files: [] }, null, 'workdir')).toBeNull()
  })

  it('当前视图不读 diff（svn 节点算不出变化）', () => {
    expect(currentDiff(workdirDiff, 'workdir', null)).toBeNull()
  })
})
