/**
 * 内置广告链接净化。
 * 覆盖：
 * - http(s)://…
 * - www./m. 域名（可无路径）
 * - 无 www 的裸域名 + 路径，如 abcd.com/aaa/bbb.html（末尾数字可变）
 * 匹配到空白或中文/全角标点即停止。
 * 裸域名无路径不剔除，避免误伤 file.txt 一类文本。
 */
const URL_TAIL = String.raw`[^\s\u4e00-\u9fff\u3000-\u303f\uff00-\uffef<>"']*`
const HOST =
  String.raw`(?:[a-zA-Z0-9][-a-zA-Z0-9]*\.)+[a-zA-Z]{2,}(?::\d{1,5})?`
const BUILTIN_AD_URL_RE = new RegExp(
  String.raw`https?://` +
    URL_TAIL +
    String.raw`|(?:www|m)\.` +
    HOST +
    String.raw`(?:\/` +
    URL_TAIL +
    String.raw`)?|` +
    HOST +
    String.raw`\/` +
    URL_TAIL,
  'gi'
)

/**
 * 应用内置净化（广告 URL 等），不依赖用户规则。
 * @param text 待净化正文
 */
export function applyBuiltinPurify(text: string): string {
  return (text || '').replace(BUILTIN_AD_URL_RE, '')
}

/**
 * 先走内置净化，再按用户规则做字面量子串删除（非正则）。
 * @param text 待净化正文
 * @param rules 要删除的子串列表
 * @returns 删除匹配片段后的文本
 */
export function applyPurify(text: string, rules: string[]): string {
  let out = applyBuiltinPurify(text)
  for (const rule of rules || []) {
    const r = (rule || '').trim()
    if (!r) continue
    if (out.includes(r)) out = out.split(r).join('')
  }
  return out
}
