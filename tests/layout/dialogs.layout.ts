import { expect, test, type Page } from '@playwright/test'
import { SCENARIOS } from './scenarios'

// 800×600 是 tauri.conf.json 的默认窗口，800×400 是用户把窗口拖矮的情况
const SIZES = [
  { width: 800, height: 600 },
  { width: 800, height: 400 },
]

async function open(page: Page, id: string, size: { width: number; height: number }) {
  await page.setViewportSize(size)
  await page.goto(`/tests/layout/harness.html?scenario=${id}`)
  await expect(page.locator('.dialog')).toBeVisible()
}

/** 量框、按钮和滚动容器。按钮用 elementFromPoint：在视口里但被盖住也算够不着。 */
function measure(page: Page) {
  return page.evaluate(() => {
    const box = document.querySelector('.dialog') as HTMLElement
    const r = box.getBoundingClientRect()
    const buttons = [...box.querySelectorAll<HTMLElement>('.dialog-actions button')].map((b) => {
      const q = b.getBoundingClientRect()
      const hit = document.elementFromPoint(q.left + q.width / 2, q.top + q.height / 2)
      return { text: b.textContent, reachable: hit === b || b.contains(hit) }
    })
    const scrollers = [box, ...box.querySelectorAll<HTMLElement>('*')].filter(
      (e) =>
        e.scrollHeight > e.clientHeight + 1 &&
        ['auto', 'scroll'].includes(getComputedStyle(e).overflowY),
    )
    return {
      top: r.top,
      bottom: r.bottom,
      viewport: innerHeight,
      buttons,
      scrollers: scrollers.length,
      horizontal: box.scrollWidth > box.clientWidth,
    }
  })
}

for (const size of SIZES) {
  for (const s of SCENARIOS) {
    test.describe(`${s.id} @ ${size.width}x${size.height}`, () => {
      test.beforeEach(async ({ page }) => {
        await open(page, s.id, size)
        if (s.clickPrimary) {
          // 从滚动区顶部点：按钮贴底以后这是最常见的点法，错误不能出在视野外
          await page.evaluate(() => {
            for (const e of document.querySelectorAll<HTMLElement>('.dialog, .dialog *')) e.scrollTop = 0
          })
          await page.locator('.dialog-actions button.primary').click()
          await expect(page.locator('.dlg-warning').last()).toBeVisible()
        }
      })

      if (s.clickPrimary) {
        test('点了主按钮之后，错误就在眼前，不被按钮条盖住', async ({ page }) => {
          const error = page.locator('.dlg-warning').last()
          await expect(error).toBeInViewport()
          const covered = await error.evaluate((e) => {
            const actions = document.querySelector('.dialog-actions')!.getBoundingClientRect()
            return e.getBoundingClientRect().top >= actions.top
          })
          expect(covered).toBe(false)
        })
      }

      test('打开时有焦点的控件不被按钮条盖住', async ({ page }) => {
        const r = await page.evaluate(() => {
          const a = document.activeElement as HTMLElement | null
          const actions = document.querySelector('.dialog-actions') as HTMLElement
          if (!a || actions.contains(a) || !a.closest('.dialog')) return null
          return { bottom: a.getBoundingClientRect().bottom, actions: actions.getBoundingClientRect().top }
        })
        if (r) expect(r.bottom).toBeLessThanOrEqual(r.actions + 1)
      })

      test('框不出窗口，每个按钮都点得到', async ({ page }) => {
        const m = await measure(page)
        expect(m.top).toBeGreaterThanOrEqual(0)
        expect(m.bottom).toBeLessThanOrEqual(m.viewport)
        expect(m.buttons.length).toBeGreaterThan(0)
        expect(m.buttons.filter((b) => !b.reachable)).toEqual([])
        expect(m.horizontal).toBe(false)
      })

      test('内容确实超长，滚轮滚得动，按钮不跟着走', async ({ page }) => {
        // 不超长就什么也没测到（AGENTS.md §8：测试的输入要是实际场景的形态）
        expect((await measure(page)).scrollers).toBeGreaterThan(0)

        const rect = await page.locator('.dialog').boundingBox()
        await page.mouse.move(rect!.x + rect!.width / 2, rect!.y + rect!.height / 3)
        await page.mouse.wheel(0, 400)
        await expect
          .poll(() =>
            page.evaluate(() =>
              [...document.querySelectorAll<HTMLElement>('.dialog, .dialog *')].reduce(
                (sum, e) => sum + e.scrollTop,
                0,
              ),
            ),
          )
          .toBeGreaterThan(0)

        const m = await measure(page)
        expect(m.buttons.filter((b) => !b.reachable)).toEqual([])
      })

      test('框内的文件清单不被压扁', async ({ page }) => {
        // 滚动容器能缩到 0：交付确认时清单只剩一条缝，等于看不到要提交什么
        const heights = await page.evaluate(() =>
          [...document.querySelectorAll<HTMLElement>('.deliver-files, .deliver-local-only')].map(
            (e) => Math.min(e.clientHeight, e.scrollHeight),
          ),
        )
        for (const h of heights) expect(h).toBeGreaterThanOrEqual(60)
      })

      test('滚到底时最后的内容不被按钮盖住', async ({ page }) => {
        const tail = await page.evaluate(() => {
          const box = document.querySelector('.dialog') as HTMLElement
          const actions = box.querySelector('.dialog-actions') as HTMLElement
          for (const e of [box, ...box.querySelectorAll<HTMLElement>('.dialog-body')]) {
            e.scrollTop = e.scrollHeight
          }
          return {
            content: (actions.previousElementSibling as HTMLElement).getBoundingClientRect().bottom,
            actions: actions.getBoundingClientRect().top,
          }
        })
        expect(tail.content).toBeLessThanOrEqual(tail.actions + 1)
      })
    })
  }
}
