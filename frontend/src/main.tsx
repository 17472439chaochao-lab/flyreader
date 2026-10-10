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

// index.html 已写入 data-os；这里再兜底一次（非 HTML 入口时）
if (!document.documentElement.dataset.os) {
  document.documentElement.dataset.os = /Mac|iPhone|iPad/.test(navigator.userAgent)
    ? 'macos'
    : /Windows/.test(navigator.userAgent)
      ? 'windows'
      : 'linux'
}

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
