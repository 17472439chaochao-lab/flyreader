import { MantineProvider } from '@mantine/core'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import { fly } from './tauri/shim'
import { appColorSchemeManager, mantineCssVariables, mantineTheme } from './theme/mantine'
// 顺序要紧：Mantine 的组件样式在前，styles.css 的同级选择器才能覆盖它
import '@mantine/core/styles.css'
import './styles.css'

window.fly = fly

// 提前打 OS 标记：macOS 用原生 Overlay 红绿灯；Windows/Linux 用自定义无边框顶栏。
const os = /Mac|iPhone|iPad/.test(navigator.userAgent)
  ? 'macos'
  : /Windows/.test(navigator.userAgent)
    ? 'windows'
    : 'linux'
document.documentElement.dataset.os = os

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <MantineProvider
      theme={mantineTheme}
      cssVariablesResolver={mantineCssVariables}
      colorSchemeManager={appColorSchemeManager}
      defaultColorScheme="light"
    >
      <App />
    </MantineProvider>
  </StrictMode>
)
