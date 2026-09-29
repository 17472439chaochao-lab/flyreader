/**
 * 规范化作者名：去掉前缀「作者：」等，便于统一展示。
 * @param author 原始作者字符串，可为空
 * @returns 清洗后的作者名；为空时返回「佚名」
 */
export function authorName(author?: string | null): string {
  const raw = (author || '').trim()
  if (!raw) return '佚名'
  return raw.replace(/^(作者|作者名|著者)[:：\s]*/, '').trim() || '佚名'
}

/**
 * 生成带统一前缀的作者展示文案。
 * @param author 原始作者字符串，可为空
 * @returns 形如「作者：xxx」的标签文本
 */
export function authorLabel(author?: string | null): string {
  return `作者：${authorName(author)}`
}
