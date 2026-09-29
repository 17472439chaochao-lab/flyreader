/**
 * 检测当前运行平台标识。
 * @returns 平台字符串，如 darwin、win32、linux
 */
function detectPlatform(): string {
  try {
    if (typeof process !== 'undefined' && process.platform) return process.platform
  } catch {
    /* 忽略 */
  }
  if (typeof navigator !== 'undefined') {
    const ua = navigator.userAgent || ''
    if (/Mac|iPhone|iPad|iPod/i.test(ua)) return 'darwin'
    if (/Win/i.test(ua)) return 'win32'
  }
  return 'linux'
}

/**
 * 按操作系统返回默认 CSS font-family 栈（应用默认字体）。
 * @param platform 平台标识，默认自动检测
 * @returns 适用于该平台的 font-family CSS 值
 */
export function platformDefaultFontFamily(platform: string = detectPlatform()): string {
  if (platform === 'darwin') {
    return '"PingFang SC", "Hiragino Sans GB", "Songti SC", sans-serif'
  }
  if (platform === 'win32') {
    return '"Microsoft YaHei", "SimSun", "Segoe UI", sans-serif'
  }
  return '"Noto Sans CJK SC", "WenQuanYi Micro Hei", "Source Han Sans SC", sans-serif'
}

/**
 * 将单个系统字体族名转为可用于 CSS 的引号形式。
 * @param family 字体族名或特殊值 system/default
 * @returns 带引号的 CSS font-family 片段，或平台默认栈
 */
export function cssFontFamily(family: string): string {
  const name = family.trim().replace(/^["']|["']$/g, '')
  if (!name) return platformDefaultFontFamily()
  if (name === 'system' || name === 'default') return platformDefaultFontFamily()
  return `"${name.replace(/\\/g, '').replace(/"/g, '')}"`
}

/**
 * 从 CSS font-family 栈中提取主字体族名。
 * @param stack font-family 字符串
 * @returns 第一个字体族名（去掉引号）
 */
export function primaryFontFamily(stack: string): string {
  const first = (stack || '').split(',')[0]?.trim() || ''
  return first.replace(/^["']|["']$/g, '')
}

const LEGACY_PRESET_STACKS = new Set([
  '"Songti SC", "Noto Serif SC", Georgia, serif',
  '"Songti SC", "Noto Serif SC", "SimSun", Georgia, serif',
  '"PingFang SC", "Heiti SC", "Microsoft YaHei", "Noto Sans SC", sans-serif',
  '"Kaiti SC", "STKaiti", "KaiTi", serif',
  '"STFangsong", "FangSong", serif',
  '"Yuanti SC", "STYuanti", "PingFang SC", sans-serif'
])

/**
 * 判断字体配置是否为旧版预设字体栈（需迁移时用）。
 * @param fontFamily 当前保存的 fontFamily 值
 * @returns 为空或属于旧预设则返回 true
 */
export function isLegacyPresetFont(fontFamily: string | undefined): boolean {
  if (!fontFamily) return true
  return LEGACY_PRESET_STACKS.has(fontFamily)
}
