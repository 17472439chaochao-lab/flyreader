import { memo, useEffect, useState } from 'react'
import { APP_ABOUT } from '@shared/about'
import type { ConfirmOutcome } from '../components/ConfirmDialog'

/**
 * 关于页：展示应用信息，并提供「检查更新」（Gitee Releases 轻量提示，不自动安装）。
 */
export const AboutView = memo(function AboutView({
  showToast,
  askConfirm
}: {
  showToast: (msg: string) => void
  askConfirm: (opts: {
    title: string
    message: string
    confirmText?: string
    cancelText?: string
  }) => Promise<ConfirmOutcome>
}) {
  const [version, setVersion] = useState<string>(APP_ABOUT.version)
  const [checking, setChecking] = useState(false)

  useEffect(() => {
    void window.fly.getVersion().then((v) => {
      if (v) setVersion(v)
    })
  }, [])

  async function handleCheckUpdate() {
    if (checking) return
    setChecking(true)
    try {
      const result = await window.fly.checkUpdate()
      if (result.status === 'available') {
        const action = await askConfirm({
          title: '发现新版本',
          message: `当前 v${result.currentVersion}，最新 v${result.latestVersion}。是否打开发行版页面下载？`,
          confirmText: '前往下载',
          cancelText: '稍后'
        })
        if (action === 'confirm') {
          const open = await window.fly.openExternal(result.releaseUrl)
          if (!open.ok) showToast(open.message || '无法打开链接')
        }
      } else if (result.status === 'upToDate') {
        showToast(`已是最新版本 v${result.currentVersion}`)
      } else {
        showToast(result.message)
      }
    } catch (e) {
      showToast(`检查更新失败：${(e as Error).message}`)
    } finally {
      setChecking(false)
    }
  }

  return (
    <div>
      <div className="panel-head">
        <div>
          <h2>关于</h2>
          <p>{APP_ABOUT.tagline}</p>
        </div>
        <button className="btn" disabled={checking} onClick={() => void handleCheckUpdate()}>
          {checking ? '检查中…' : '检查更新'}
        </button>
      </div>
      <div className="about-card">
        <div className="about-brand">
          <strong>{APP_ABOUT.name}</strong>
          <span>v{version}</span>
        </div>
        <p className="about-features">{APP_ABOUT.features}</p>
        <dl className="about-meta">
          <div>
            <dt>作者</dt>
            <dd>{APP_ABOUT.author}</dd>
          </div>
          <div>
            <dt>QQ</dt>
            <dd>{APP_ABOUT.qq}</dd>
          </div>
          <div>
            <dt>邮箱</dt>
            <dd>
              <a href={`mailto:${APP_ABOUT.email}`}>{APP_ABOUT.email}</a>
            </dd>
          </div>
          <div>
            <dt>仓库</dt>
            <dd>
              <a href={APP_ABOUT.repo} target="_blank" rel="noreferrer">
                {APP_ABOUT.repo}
              </a>
            </dd>
          </div>
          <div>
            <dt>许可证</dt>
            <dd>{APP_ABOUT.license}</dd>
          </div>
        </dl>
        <p className="about-note">{APP_ABOUT.opensourceNote}</p>
        <h4>开源组件</h4>
        <ul className="about-deps">
          {APP_ABOUT.components.map((c) => (
            <li key={c.name}>
              <strong>{c.name}</strong>
              <span>{c.desc}</span>
              <em>{c.license}</em>
            </li>
          ))}
        </ul>
      </div>
    </div>
  )
})
