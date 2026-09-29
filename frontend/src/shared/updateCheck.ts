/** 轻量更新检查结果（方案 1：仅提示，不自动下载安装）。 */
export type UpdateCheckResult =
  | {
      status: 'available'
      currentVersion: string
      latestVersion: string
      releaseUrl: string
      releaseName?: string
      releaseNotes?: string
    }
  | {
      status: 'upToDate'
      currentVersion: string
      latestVersion: string
    }
  | {
      status: 'noRelease'
      currentVersion: string
      message: string
    }
  | {
      status: 'error'
      currentVersion: string
      message: string
    }

/**
 * 去掉前缀 v/V 并裁剪空白，得到可比较的版本串。
 * @param version 原始版本号，如 v1.0.1
 */
export function normalizeVersion(version: string): string {
  return (version || '').trim().replace(/^v/i, '')
}

/**
 * 比较两个语义化风格版本号（按数字段比较）。
 * @param a 版本 A
 * @param b 版本 B
 * @returns a>b 为正，a<b 为负，相等为 0
 */
export function compareVersions(a: string, b: string): number {
  const pa = normalizeVersion(a)
    .split(/[.+-]/)
    .map((x) => {
      const n = parseInt(x, 10)
      return Number.isFinite(n) ? n : 0
    })
  const pb = normalizeVersion(b)
    .split(/[.+-]/)
    .map((x) => {
      const n = parseInt(x, 10)
      return Number.isFinite(n) ? n : 0
    })
  const len = Math.max(pa.length, pb.length)
  for (let i = 0; i < len; i++) {
    const d = (pa[i] || 0) - (pb[i] || 0)
    if (d !== 0) return d
  }
  return 0
}
