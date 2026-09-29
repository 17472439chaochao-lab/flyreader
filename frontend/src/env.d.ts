import type { FlyApi } from './tauri/shim'

declare global {
  interface Window {
    /** tauri/shim.ts 在 main.tsx 中挂载的 FlyReader API（原 Electron preload 桥的替代） */
    fly: FlyApi
  }
}

export {}
