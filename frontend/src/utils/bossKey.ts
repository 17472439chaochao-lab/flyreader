import type { KeyboardEvent } from 'react'

/**
 * 将 Electron accelerator 转为更易读的展示文案。
 * @param accel 如 CommandOrControl+Shift+H
 * @returns 替换修饰键后的标签，如 Ctrl + Shift + H
 */
export function formatBossKeyLabel(accel: string): string {
  return accel
    .replace(/CommandOrControl/g, 'Ctrl')
    .replace(/Command/g, '⌘')
    .replace(/Control/g, 'Ctrl')
    .replace(/Option|Alt/g, 'Alt')
    .replace(/Shift/g, 'Shift')
    .replace(/\+/g, ' + ')
}

/**
 * 将键盘事件转为 Electron 全局快捷键 accelerator。
 * 至少需要一个修饰键，单独按键返回 null。
 * @param e React 键盘事件
 * @returns accelerator 字符串，或无法构成安全热键时返回 null
 */
export function eventToAccelerator(e: KeyboardEvent): string | null {
  const key = e.key
  if (!key || key === 'Shift' || key === 'Control' || key === 'Alt' || key === 'Meta') return null

  const parts: string[] = []
  // 优先使用 Electron globalShortcut 的跨平台修饰键
  if (e.metaKey || e.ctrlKey) parts.push('CommandOrControl')
  if (e.altKey) parts.push('Alt')
  if (e.shiftKey) parts.push('Shift')

  let code = key
  if (key === ' ') code = 'Space'
  else if (key === 'ArrowUp') code = 'Up'
  else if (key === 'ArrowDown') code = 'Down'
  else if (key === 'ArrowLeft') code = 'Left'
  else if (key === 'ArrowRight') code = 'Right'
  else if (key === 'Escape') code = 'Esc'
  else if (key.length === 1) code = key.toUpperCase()
  else if (/^F\d{1,2}$/i.test(key)) code = key.toUpperCase()

  // 全局热键至少需要一个修饰键
  if (!parts.length) return null
  parts.push(code)
  return parts.join('+')
}
