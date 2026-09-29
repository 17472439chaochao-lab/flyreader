import { memo, useEffect, useState } from 'react'
import {
  EYE_CARE_INTERVAL_OPTIONS,
  REQUEST_CONCURRENCY_MAX,
  REQUEST_CONCURRENCY_MIN,
  type AppPrefs,
  type ReaderSettings
} from '@shared/types'
import { IconTrash } from '../components/icons'
import { eventToAccelerator, formatBossKeyLabel } from '../utils/bossKey'
import type { LegacyDataStatus } from '../tauri/shim'

/** 旧版数据体量提示 */
function formatBytes(bytes: number) {
  if (bytes <= 0) return '0 MB'
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

/**
 * 设置页：老板键、护眼提醒、阅读预加载、网络并发、正文净化规则与旧版数据导入。
 */
export const SettingsView = memo(function SettingsView({
  settings,
  prefs,
  askConfirm,
  onPrefsChange,
  onRemovePurify,
  onLegacyStatus,
  onImportLegacy
}: {
  settings: ReaderSettings
  prefs: AppPrefs
  askConfirm: (opts: {
    title: string
    message: string
    confirmText?: string
    cancelText?: string
    extraText?: string
    danger?: boolean
  }) => Promise<'confirm' | 'cancel' | 'extra'>
  onPrefsChange: (patch: Partial<AppPrefs>) => void | Promise<void>
  onRemovePurify: (rule: string) => void
  onLegacyStatus: () => Promise<LegacyDataStatus>
  onImportLegacy: () => Promise<{ merged: { shelf: number; sources: number } }>
}) {
  const rules = settings.purifyRules || []
  const [bossKeyDraft, setBossKeyDraft] = useState(prefs.bossKey)
  const [recordingBossKey, setRecordingBossKey] = useState(false)
  const [legacy, setLegacy] = useState<LegacyDataStatus | null>(null)
  const [importing, setImporting] = useState(false)
  const [importError, setImportError] = useState('')
  useEffect(() => {
    setBossKeyDraft(prefs.bossKey)
  }, [prefs.bossKey])
  useEffect(() => {
    void onLegacyStatus().then(setLegacy)
  }, [onLegacyStatus])

  const bossLabel = formatBossKeyLabel(bossKeyDraft || prefs.bossKey)

  async function importLegacyNow() {
    const choice = await askConfirm({
      title: '导入旧版数据',
      message: `将旧版 FlyNovel 的书架、书源、设置与已缓存章节合并到 FlyReader。\n\n${legacy?.path ?? ''}（约 ${formatBytes(legacy?.bytes ?? 0)}）\n\n同 ID 条目会被旧版内容覆盖；旧版数据库保持不变，可重复导入。`,
      confirmText: '开始导入'
    })
    if (choice !== 'confirm') return
    setImporting(true)
    setImportError('')
    try {
      await onImportLegacy()
    } catch (e) {
      setImportError(e instanceof Error ? e.message : String(e))
    } finally {
      setImporting(false)
    }
  }

  return (
    <div>
      <div className="panel-head">
        <div>
          <h2>老板键</h2>
          <p>一键隐藏 / 显示窗口，避免旁人窥屏</p>
        </div>
      </div>
      <div className="settings-form">
        <div className="field row-field">
          <label>启用老板键</label>
          <button
            type="button"
            className={`switch ${prefs.bossKeyEnabled ? 'on' : ''}`}
            onClick={() => void onPrefsChange({ bossKeyEnabled: !prefs.bossKeyEnabled })}
          >
            <i />
          </button>
        </div>
        <div className="field">
          <label>老板键快捷键</label>
          <button
            type="button"
            className={`boss-key-capture ${recordingBossKey ? 'recording' : ''}`}
            onClick={() => setRecordingBossKey(true)}
            onBlur={() => {
              setRecordingBossKey(false)
              const next = bossKeyDraft.trim()
              if (next && next !== prefs.bossKey) void onPrefsChange({ bossKey: next })
            }}
            onKeyDown={(e) => {
              if (!recordingBossKey && e.key !== 'Enter' && e.key !== ' ') return
              e.preventDefault()
              e.stopPropagation()
              if (e.key === 'Escape') {
                setRecordingBossKey(false)
                setBossKeyDraft(prefs.bossKey)
                return
              }
              const accel = eventToAccelerator(e)
              if (!accel) return
              setBossKeyDraft(accel)
              setRecordingBossKey(false)
              if (accel !== prefs.bossKey) void onPrefsChange({ bossKey: accel })
            }}
          >
            {recordingBossKey ? '请按下组合键…' : bossLabel || '点击设置'}
          </button>
          <p className="field-hint">点击上方区域后按下组合键（需含 Ctrl / ⌘ / Alt / Shift），Esc 取消</p>
        </div>
      </div>

      <div className="panel-head" style={{ marginTop: 28 }}>
        <div>
          <h2>护眼提醒</h2>
          <p>连续阅读达到设定时长后提示休息；老板键隐藏时不计时、不弹窗</p>
        </div>
      </div>
      <div className="settings-form">
        <div className="field row-field">
          <label>启用护眼提醒</label>
          <button
            type="button"
            className={`switch ${prefs.eyeCareEnabled ? 'on' : ''}`}
            onClick={() => void onPrefsChange({ eyeCareEnabled: !prefs.eyeCareEnabled })}
          >
            <i />
          </button>
        </div>
        <div className="field">
          <label>提醒间隔</label>
          <select
            value={prefs.eyeCareIntervalMinutes ?? 120}
            disabled={!prefs.eyeCareEnabled}
            onChange={(e) =>
              void onPrefsChange({ eyeCareIntervalMinutes: Number(e.target.value) })
            }
            aria-label="护眼提醒间隔"
          >
            {EYE_CARE_INTERVAL_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
          <p className="field-hint">仅在阅读页累计时长；离开阅读或隐藏窗口时暂停</p>
        </div>
      </div>

      <div className="panel-head" style={{ marginTop: 28 }}>
        <div>
          <h2>阅读预加载</h2>
          <p>阅读时自动预取后续章节，减少翻页等待</p>
        </div>
      </div>
      <div className="settings-form">
        <div className="field">
          <label>预加载后续章节（{prefs.preloadCount} 章）</label>
          <input
            type="range"
            min={0}
            max={10}
            step={1}
            value={prefs.preloadCount}
            onChange={(e) => void onPrefsChange({ preloadCount: Number(e.target.value) })}
          />
          <p className="field-hint">设为 0 可关闭预加载；已缓存章节会跳过</p>
        </div>
      </div>

      <div className="panel-head" style={{ marginTop: 28 }}>
        <div>
          <h2>网络请求并发</h2>
          <p>控制同时发出的网络请求数量，过高可能被站点限流</p>
        </div>
      </div>
      <div className="settings-form">
        <div className="field">
          <label>并发数（{prefs.requestConcurrency ?? REQUEST_CONCURRENCY_MIN}）</label>
          <input
            type="range"
            min={REQUEST_CONCURRENCY_MIN}
            max={REQUEST_CONCURRENCY_MAX}
            step={1}
            value={prefs.requestConcurrency ?? REQUEST_CONCURRENCY_MIN}
            onChange={(e) => void onPrefsChange({ requestConcurrency: Number(e.target.value) })}
          />
          <p className="field-hint">
            搜索、书源测试、缓存、更新与导出共用；范围 {REQUEST_CONCURRENCY_MIN}–
            {REQUEST_CONCURRENCY_MAX}
          </p>
        </div>
      </div>

      <div className="panel-head" style={{ marginTop: 28 }}>
        <div>
          <h2>正文净化</h2>
          <p>
            已内置剔除正文中的广告链接：http(s)://、www./m. 域名，以及不带 www 但带路径的裸域名（如 site.com/a/123.html）。阅读页选中文字后右键可追加自定义规则
          </p>
        </div>
      </div>
      {!rules.length ? (
        <div className="empty">暂无净化规则</div>
      ) : (
        <div className="purify-list">
          {rules.map((rule) => (
            <div className="purify-row" key={rule}>
              <code>{rule}</code>
              <button
                className="btn ghost icon-btn"
                title="删除"
                aria-label="删除"
                onClick={() => {
                  void (async () => {
                    const result = await askConfirm({
                      title: '删除净化规则',
                      message: `确定删除这条净化规则？\n\n${rule}`,
                      confirmText: '删除',
                      danger: true
                    })
                    if (result !== 'confirm') return
                    onRemovePurify(rule)
                  })()
                }}
              >
                <IconTrash />
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="panel-head" style={{ marginTop: 28 }}>
        <div>
          <h2>旧版数据</h2>
          <p>把 Electron 版 FlyNovel 的书架、书源、设置与已缓存章节合并进来；旧版数据保持不变</p>
        </div>
      </div>
      {legacy === null ? (
        <div className="empty">正在检测旧版数据…</div>
      ) : legacy.available ? (
        <div className="settings-form">
          <div className="field">
            <label>检测到旧版数据库（约 {formatBytes(legacy.bytes)}）</label>
            <button
              type="button"
              className="btn"
              disabled={importing}
              onClick={() => void importLegacyNow()}
            >
              {importing ? '正在导入…' : '导入旧版数据'}
            </button>
            <p className="field-hint">
              {importError || legacy.path}
              {importError ? '' : '；数据量大时可能需要数十秒'}
            </p>
          </div>
        </div>
      ) : (
        <div className="empty">未检测到旧版 FlyNovel 数据</div>
      )}
    </div>
  )
})
