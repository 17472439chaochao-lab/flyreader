import { resolve } from 'node:path'
import { defineConfig } from 'vitest/config'

// 前端单测：jsdom 环境，覆盖 frontend/src 下的纯函数与 XSS 回归用例
export default defineConfig({
  resolve: {
    alias: {
      '@shared': resolve(__dirname, 'frontend/src/shared')
    }
  },
  test: {
    environment: 'jsdom',
    include: ['frontend/src/**/*.test.ts', 'frontend/src/**/*.test.tsx'],
    globals: false
  }
})
