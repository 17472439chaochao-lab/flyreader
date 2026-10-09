/** 详情取空时的书名占位，不当作有效元信息。 */
const PLACEHOLDER_NAME = '未知书名'
/** 详情取空时的作者占位，不当作有效元信息。 */
const PLACEHOLDER_AUTHOR = '佚名'

/**
 * 从候选里挑有效书名；跳过空串与「未知书名」，避免盖掉搜索/书架已有真名。
 */
export function pickBookName(
  ...candidates: Array<string | null | undefined>
): string {
  for (const c of candidates) {
    const v = (c || '').trim()
    if (v && v !== PLACEHOLDER_NAME) return v
  }
  for (const c of candidates) {
    const v = (c || '').trim()
    if (v) return v
  }
  return PLACEHOLDER_NAME
}

/**
 * 从候选里挑有效作者；跳过空串与「佚名」。
 */
export function pickBookAuthor(
  ...candidates: Array<string | null | undefined>
): string {
  for (const c of candidates) {
    const v = (c || '').trim()
    if (v && v !== PLACEHOLDER_AUTHOR) return v
  }
  for (const c of candidates) {
    const v = (c || '').trim()
    if (v) return v
  }
  return PLACEHOLDER_AUTHOR
}
