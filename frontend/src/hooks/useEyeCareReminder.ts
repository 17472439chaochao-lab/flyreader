import { useEffect, useRef } from 'react'
import { EYE_CARE_INTERVAL_OPTIONS, eyeCareIntervalMs } from '@shared/types'
import type { ConfirmOutcome } from '../components/ConfirmDialog'

/**
 * 将分钟数格式化为中文时长文案。
 * @param minutes 分钟
 */
export function formatEyeCareDuration(minutes: number): string {
  const opt = EYE_CARE_INTERVAL_OPTIONS.find((o) => o.value === minutes)
  if (opt) return opt.label
  if (minutes < 60) return `${minutes} 分钟`
  const h = Math.floor(minutes / 60)
  const m = minutes % 60
  return m ? `${h} 小时 ${m} 分钟` : `${h} 小时`
}

/**
 * 护眼 / 久坐提醒：仅在阅读页且窗口可见时累计时长；
 * 老板键隐藏时暂停计时并关闭提醒，避免旁人察觉。
 */
export function useEyeCareReminder(opts: {
  enabled: boolean
  intervalMinutes: number
  /** 是否处于阅读页 */
  readingActive: boolean
  askConfirm: (opts: {
    title: string
    message: string
    confirmText?: string
    cancelText?: string
  }) => Promise<ConfirmOutcome>
  /** 老板键隐藏时主动关闭对话框 */
  dismissConfirm: () => void
}): void {
  const { enabled, intervalMinutes, readingActive, askConfirm, dismissConfirm } = opts
  const accumulatedMsRef = useRef(0)
  const lastTickRef = useRef<number | null>(null)
  const bossHiddenRef = useRef(false)
  const promptingRef = useRef(false)
  const askConfirmRef = useRef(askConfirm)
  const dismissConfirmRef = useRef(dismissConfirm)
  askConfirmRef.current = askConfirm
  dismissConfirmRef.current = dismissConfirm

  useEffect(() => {
    const off = window.fly.onBossVisibility(({ hidden }) => {
      bossHiddenRef.current = hidden
      if (hidden) {
        lastTickRef.current = null
        if (promptingRef.current) {
          promptingRef.current = false
          dismissConfirmRef.current()
        }
      }
    })
    return off
  }, [])

  useEffect(() => {
    if (!enabled) {
      accumulatedMsRef.current = 0
      lastTickRef.current = null
      return
    }

    const intervalMs = eyeCareIntervalMs(intervalMinutes)

    const tick = () => {
      if (bossHiddenRef.current || promptingRef.current) {
        lastTickRef.current = null
        return
      }
      if (!readingActive || document.visibilityState !== 'visible') {
        lastTickRef.current = null
        return
      }

      const now = Date.now()
      if (lastTickRef.current != null) {
        accumulatedMsRef.current += Math.max(0, now - lastTickRef.current)
      }
      lastTickRef.current = now

      if (accumulatedMsRef.current < intervalMs || promptingRef.current) return

      promptingRef.current = true
      lastTickRef.current = null
      const label = formatEyeCareDuration(intervalMinutes)
      void askConfirmRef
        .current({
          title: '护眼提醒',
          message: `您已连续阅读约 ${label}，请起身活动、眺望远方，让眼睛休息一下。`,
          confirmText: '知道了',
          cancelText: '稍后'
        })
        .then(() => {
          accumulatedMsRef.current = 0
          promptingRef.current = false
          lastTickRef.current = null
        })
    }

    const id = window.setInterval(tick, 1000)
    tick()
    const onVis = () => {
      if (document.visibilityState !== 'visible') lastTickRef.current = null
    }
    document.addEventListener('visibilitychange', onVis)
    return () => {
      window.clearInterval(id)
      document.removeEventListener('visibilitychange', onVis)
      lastTickRef.current = null
    }
  }, [enabled, intervalMinutes, readingActive])
}
