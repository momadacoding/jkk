import { defineConfig } from '@playwright/test'

/**
 * 布局回归：真实量弹框的按钮在不在窗口里（AGENTS.md §6.1）。jsdom 不做布局，这类缺陷只能在这里抓。
 *
 * - `*.layout.ts` 而不是 `*.spec.ts`：后者会被 vitest 默认的 include 收走。
 * - 有头 + msedge：headless 的滚动条与真实窗口不同（§6.1 禁止），WebView2 就是 Edge 内核。
 * - 端口交给系统分配（`--port 0`）。开发机的动态端口范围覆盖到了五千多，写死的端口会被别的
 *   进程的出站连接占掉，报 EACCES（2026-11-03 实测）；也不会测到别的 worktree 开的服务器。
 *   `wait` 从 vite 的输出里取回端口，写进 `HARNESS_PORT`，worker 重新加载本文件时读到。
 */
export default defineConfig({
  testDir: 'tests/layout',
  testMatch: '*.layout.ts',
  outputDir: 'test-results',
  // 有头窗口，一次一个，不和用户抢屏幕
  workers: 1,
  reporter: 'list',
  use: {
    baseURL: `http://127.0.0.1:${process.env.HARNESS_PORT}`,
    channel: 'msedge',
    headless: false,
    actionTimeout: 3000,
  },
  webServer: {
    command: 'pnpm exec vite --port 0',
    // 端口数字前可能夹着颜色转义
    wait: { stdout: /127\.0\.0\.1:(?:\x1b\[\d+m)*(?<harness_port>\d+)/ },
    reuseExistingServer: false,
  },
})
