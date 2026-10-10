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

// 沉浸式标题栏只存在于 macOS；提前在首帧前打标记，让 CSS 归零原生标题栏下多余的空带。
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
