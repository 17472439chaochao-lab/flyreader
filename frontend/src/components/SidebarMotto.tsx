import { useEffect, useState } from 'react'
import { pickSidebarMotto } from '../utils/sidebarMottos'

const ROTATE_MS = 5 * 60 * 1000

/**
 * 侧栏底部励志短句：按时段轮换，轻量展示。
 */
export function SidebarMotto() {
  const [text, setText] = useState(() => pickSidebarMotto())

  useEffect(() => {
    const tick = () => setText(pickSidebarMotto())
    tick()
    const id = window.setInterval(tick, ROTATE_MS)
    return () => window.clearInterval(id)
  }, [])

  return (
    <div className="sidebar-motto" aria-live="polite">
      <p>{text}</p>
    </div>
  )
}
