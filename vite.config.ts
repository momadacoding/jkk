import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'node:path'

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  clearScreen: false,
  server: {
    /**
     * 明确绑到 127.0.0.1，不要用默认的 `localhost`。
     *
     * `localhost` 在 Windows 上先解析成 `::1`，Node 17+ 又按系统顺序原样使用，
     * 于是 vite 只监听 IPv6；而 Tauri 探测 devUrl 走的是 IPv4，永远连不上，
     * 卡在 "Waiting for your frontend dev server to start..." 不动。
     * tauri.conf.json 的 devUrl 也一并写成 127.0.0.1，两边不留解析歧义。
     *
     * TAURI_DEV_HOST 是真机调试用的（官方模板的做法），设了就听它的。
     */
    host: process.env.TAURI_DEV_HOST || '127.0.0.1',
    port: 5174,
    strictPort: true,
  },
})
