import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'

/** 运行时判断，避免模块加载早于 main.tsx 写入 data-os 时被冻成 false。 */
function useCustomChrome(): boolean {
  const [enabled] = useState(() => {
    const os = document.documentElement.dataset.os
    if (os === 'windows' || os === 'linux') return true
    const ua = navigator.userAgent
    return /Windows/i.test(ua) || (/Linux/i.test(ua) && !/Android/i.test(ua))
  })
  return enabled
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  const tag = target.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable
}

/**
 * Windows / Linux 自定义标题栏控件：去掉原生菜单栏后补齐最小化 / 最大化 / 关闭，
 * 并承接原菜单里的重载与缩放快捷键。
 */
export function WindowChrome() {
  const enabled = useCustomChrome()
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    if (!enabled) return
    // 与 CSS 选择器对齐；防止模块加载顺序导致 data-os 未写上
    if (!document.documentElement.dataset.os) {
      document.documentElement.dataset.os = /Windows/i.test(navigator.userAgent)
        ? 'windows'
        : 'linux'
    }
    const win = getCurrentWindow()
    let disposed = false
    void win.isMaximized().then((v) => {
      if (!disposed) setMaximized(v)
    })
    const unlisten = win.onResized(() => {
      void win.isMaximized().then((v) => {
        if (!disposed) setMaximized(v)
      })
    })
    return () => {
      disposed = true
      void unlisten.then((off) => off())
    }
  }, [enabled])

  useEffect(() => {
    if (!enabled) return
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e.target)) return
      const mod = e.ctrlKey || e.metaKey
      if (!mod) return
      if (e.key === 'r' || e.key === 'R') {
        e.preventDefault()
        location.reload()
        return
      }
      if (e.key === '0') {
        e.preventDefault()
        void invoke('app_apply_zoom', { action: 'reset' })
        return
      }
      if (e.key === '=' || e.key === '+') {
        e.preventDefault()
        void invoke('app_apply_zoom', { action: 'in' })
        return
      }
      if (e.key === '-' || e.key === '_') {
        e.preventDefault()
        void invoke('app_apply_zoom', { action: 'out' })
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [enabled])

  if (!enabled) return null

  const win = getCurrentWindow()

  return (
    <div className="win-controls" role="toolbar" aria-label="窗口控制">
      <button
        type="button"
        className="win-ctl"
        aria-label="最小化"
        title="最小化"
        onClick={() => void win.minimize()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M1 5h8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
        </svg>
      </button>
      <button
        type="button"
        className="win-ctl"
        aria-label={maximized ? '还原' : '最大化'}
        title={maximized ? '还原' : '最大化'}
        onClick={() => void win.toggleMaximize()}
      >
        {maximized ? (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
            <path
              d="M3 2.5h4.5V7M2.5 3.5H7v4.5H2.5z"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.1"
            />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
            <rect
              x="1.5"
              y="1.5"
              width="7"
              height="7"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.2"
            />
          </svg>
        )}
      </button>
      <button
        type="button"
        className="win-ctl win-ctl-close"
        aria-label="关闭"
        title="关闭"
        onClick={() => void win.close()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path
            d="M2 2l6 6M8 2L2 8"
            stroke="currentColor"
            strokeWidth="1.2"
            strokeLinecap="round"
          />
        </svg>
      </button>
    </div>
  )
}
