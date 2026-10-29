// @vitest-environment jsdom
//
// 组件测试只在这个文件里切到 jsdom，其余纯逻辑测试仍跑在 node 环境（vitest.config.ts）。
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import SvnInitWizard from '@/components/SvnInitWizard'

function mount() {
  const never = () => new Promise<never>(() => undefined)
  render(
    <SvnInitWizard
      repoPath="D:/repo"
      onCancel={() => undefined}
      onFinish={() => undefined}
      onPreflight={never}
      onInit={never}
    />,
  )
  const input = screen.getByLabelText(/收哪些扩展名/) as HTMLInputElement
  const preview = () => screen.getByTestId('svn-ext-preview')
  return { input, preview }
}

describe('扩展名预览', () => {
  it('给出条数并逐条显示，切分结果一眼可核对', () => {
    const { input, preview } = mount()
    fireEvent.change(input, { target: { value: '.cs、.lua、.java' } })
    expect(preview().textContent).toContain('3 条')
    const items = Array.from(preview().querySelectorAll('code')).map((c) => c.textContent)
    expect(items).toEqual(['.cs', '.lua', '.java'])
  })

  it('空输入显示（空）', () => {
    const { input, preview } = mount()
    fireEvent.change(input, { target: { value: ' ，、 ' } })
    expect(preview().textContent).toContain('（空）')
    expect(preview().querySelectorAll('code')).toHaveLength(0)
  })
})
