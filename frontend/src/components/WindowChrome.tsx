import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'

const isCustomChrome =
  typeof document !== 'undefined' &&
  (document.documentElement.dataset.os === 'windows' ||
    document.documentElement.dataset.os === 'linux')

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
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    if (!isCustomChrome) return
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
  }, [])

  useEffect(() => {
    if (!isCustomChrome) return
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
  }, [])

  if (!isCustomChrome) return null

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
