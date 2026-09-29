import { resolve } from 'path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Tauri 官方约定：固定端口 1420，通过 TAURI_DEV_HOST 支持真机/外部 IP
export default defineConfig({
  root: 'frontend',
  plugins: [react()],
  clearScreen: false,
  resolve: {
    alias: {
      '@shared': resolve('frontend/src/shared')
    }
  },
  server: {
    port: 1420,
    strictPort: true,
    host: process.env.TAURI_DEV_HOST || '127.0.0.1'
  },
  build: {
    outDir: 'dist',
    target: 'es2021'
  }
})
