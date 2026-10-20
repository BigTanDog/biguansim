import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  // Tauri 在 dev 模式下按固定端口加载前端。端口被占时宁可启动失败，
  // 也不要静默换端口让 Tauri 连到别的进程上。
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    // 目标环境是 Windows WebView2（Chromium），不必为旧浏览器降级语法。
    target: 'chrome110',
    sourcemap: false,
  },
})
