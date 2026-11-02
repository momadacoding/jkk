import { expect, test } from '@playwright/test'

// 向导不是 .dialog，不走基类的高度上限，要单独量
for (const size of [
  { width: 800, height: 600 },
  { width: 800, height: 400 },
]) {
  test(`SVN 向导报长错误时按钮够得着 @ ${size.width}x${size.height}`, async ({ page }) => {
    await page.setViewportSize(size)
    await page.goto('/tests/layout/harness.html?scenario=wizard-long-error')
    await page.locator('#svn-whitelist').fill('Assets/Scripts')
    await page.getByRole('button', { name: '检查工作副本' }).click()
    await expect(page.locator('.svn-notice.error')).toBeVisible()

    const reachable = await page.evaluate(() => {
      const b = [...document.querySelectorAll<HTMLElement>('.dlg-actions button')]
      return b.map((e) => {
        const q = e.getBoundingClientRect()
        const hit = document.elementFromPoint(q.left + q.width / 2, q.top + q.height / 2)
        return { text: e.textContent, ok: hit === e || e.contains(hit), top: Math.round(q.top) }
      })
    })
    expect(reachable.length).toBeGreaterThan(0)
    expect(reachable.filter((b) => !b.ok)).toEqual([])

    // 按钮贴底之后，内容不能被它盖住够不着
    const tail = await page.evaluate(() => {
      const wizard = document.querySelector('.svn-wizard') as HTMLElement
      wizard.scrollTop = wizard.scrollHeight
      const actions = wizard.querySelector('.dlg-actions') as HTMLElement
      return {
        content: (actions.previousElementSibling as HTMLElement).getBoundingClientRect().bottom,
        actions: actions.getBoundingClientRect().top,
      }
    })
    expect(tail.content).toBeLessThanOrEqual(tail.actions + 1)
  })
}
