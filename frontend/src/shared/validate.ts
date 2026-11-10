import type { BookSource, ShelfBook } from './types'

/**
 * 轻量运行时守卫：后端返回形状不对时直接报错，而不是静默渲染残缺数据。
 * 只校验关键字段（类型与非空），不做全量 schema 校验。
 */

/** 校验单个书源的关键字段 */
export function assertBookSource(value: unknown, where = 'BookSource'): asserts value is BookSource {
  if (!value || typeof value !== 'object') throw new Error(`${where} 返回数据异常：不是对象`)
  const s = value as Partial<BookSource>
  if (typeof s.bookSourceUrl !== 'string' || !s.bookSourceUrl.trim()) {
    throw new Error(`${where} 返回数据异常：缺少 bookSourceUrl`)
  }
  if (typeof s.bookSourceName !== 'string' || !s.bookSourceName.trim()) {
    throw new Error(`${where} 返回数据异常：缺少 bookSourceName`)
  }
}

/** 校验书源列表 */
export function assertBookSourceList(value: unknown, where = 'BookSource[]'): BookSource[] {
  if (!Array.isArray(value)) throw new Error(`${where} 返回数据异常：不是数组`)
  for (const s of value) assertBookSource(s, where)
  return value as BookSource[]
}

/** 校验单个书架书籍的关键字段 */
export function assertShelfBook(value: unknown, where = 'ShelfBook'): asserts value is ShelfBook {
  if (!value || typeof value !== 'object') throw new Error(`${where} 返回数据异常：不是对象`)
  const b = value as Partial<ShelfBook>
  if (typeof b.id !== 'string' || !b.id.trim()) {
    throw new Error(`${where} 返回数据异常：缺少 id`)
  }
  if (typeof b.name !== 'string' || !b.name.trim()) {
    throw new Error(`${where} 返回数据异常：缺少 name`)
  }
}

/** 校验书架列表 */
export function assertShelfBookList(value: unknown, where = 'ShelfBook[]'): ShelfBook[] {
  if (!Array.isArray(value)) throw new Error(`${where} 返回数据异常：不是数组`)
  for (const b of value) assertShelfBook(b, where)
  return value as ShelfBook[]
}

export type BookSourceJsonError = { ok: false; message: string }

/**
 * 校验用户手写/粘贴的书源 JSON：先过关键字段与类型校验再 as BookSource 入库。
 * @param text 原始 JSON 文本
 * @param name 用户在编辑框里填的书源名称（为空则用 JSON 内的名称）
 * @returns 校验通过返回 { ok: true, source }，否则返回错误文案
 */
export function parseBookSourceJson(
  text: string,
  name: string
): { ok: true; source: BookSource } | BookSourceJsonError {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch (e) {
    return { ok: false, message: `JSON 解析失败：${e instanceof Error ? e.message : String(e)}` }
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return { ok: false, message: '书源 JSON 必须是单个对象' }
  }
  const obj = parsed as Record<string, unknown>
  const sourceName = name.trim() || (typeof obj.bookSourceName === 'string' ? obj.bookSourceName.trim() : '')
  if (!sourceName) return { ok: false, message: '书源名称不能为空' }
  if (typeof obj.bookSourceUrl !== 'string' || !obj.bookSourceUrl.trim()) {
    return { ok: false, message: '书源 URL 不能为空且必须是字符串' }
  }
  // 可选字段类型校验：类型错直接拒绝，避免脏数据入库后在别处炸开
  const stringFields = ['bookSourceGroup', 'searchUrl', 'exploreUrl', 'loginUrl', 'bookUrlPattern', 'header'] as const
  for (const f of stringFields) {
    if (obj[f] !== undefined && typeof obj[f] !== 'string') {
      return { ok: false, message: `字段 ${f} 必须是字符串` }
    }
  }
  const numberFields = ['bookSourceType', 'customOrder', 'weight', 'lastUpdateTime', 'respondTime', 'flyRespondMs', 'flyMatchScore', 'flyMatchSamples', 'flyTestAt'] as const
  for (const f of numberFields) {
    if (obj[f] !== undefined && typeof obj[f] !== 'number') {
      return { ok: false, message: `字段 ${f} 必须是数字` }
    }
  }
  if (obj.enabled !== undefined && typeof obj.enabled !== 'boolean') {
    return { ok: false, message: '字段 enabled 必须是布尔值' }
  }
  return { ok: true, source: { ...(parsed as BookSource), bookSourceName: sourceName } }
}
