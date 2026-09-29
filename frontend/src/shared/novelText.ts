/**
 * 统一换行符为 `\n`（主进程与渲染进程共用）。
 * 处理 Windows CRLF、旧 Mac CR、Unix LF 及 Unicode 行/段分隔符。
 * @param text 原始文本
 * @returns 换行已规范化的文本
 */
export function normalizeLineEndings(text: string): string {
  return text
    .replace(/\r\n/g, '\n')
    .replace(/\r/g, '\n')
    .replace(/\u2028|\u2029/g, '\n')
}

/** 打包短句为目标段落长度（约字数），避免一句一段。 */
const PACK_TARGET = 120

/**
 * 为中文网文整理可读段落。
 * 优先保留原文 / HTML 已有结构；仅对「整墙无换行」做适度拆分，
 * 不会在每个句号后强制另起一段。
 * @param text 章节正文
 * @returns 以双换行分段后的可读文本
 */
export function ensureNovelParagraphs(text: string): string {
  if (!text) return ''
  let s = normalizeLineEndings(text).trim()
  if (!s) return ''

  if (/<\/?[a-z][\s\S]*>/i.test(s)) {
    s = s.replace(/<(script|style)[^>]*>[\s\S]*?<\/\1>/gi, '')
    s = s.replace(/<br\s*\/?>/gi, '\n')
    s = s.replace(/<\/?(p|div|section|article|tr|li|h[1-6]|blockquote)(\s[^>]*)?>/gi, '\n')
    s = s.replace(/<[^>]+>/g, '')
  }

  s = s
    .replace(/&nbsp;/gi, ' ')
    .replace(/\u3000{2,}/g, '\n')
    .replace(/(^|\n)\u3000+/g, '$1')

  // 先按已有换行切成块，并合并明显「句中被截断」的短行
  const rough = s
    .split(/\n+/)
    .map((p) => p.replace(/[ \t\f\v]+/g, ' ').trim())
    .filter(Boolean)

  const merged: string[] = []
  for (const p of rough) {
    const prev = merged[merged.length - 1]
    if (
      prev &&
      !/[。！？…~～”」』'’]$/.test(prev) &&
      !/^[“「『"‘]/.test(p) &&
      prev.length < 80
    ) {
      merged[merged.length - 1] = prev + p
    } else {
      merged.push(p)
    }
  }

  const totalLen = merged.reduce((n, p) => n + p.length, 0)
  const avg = totalLen / Math.max(merged.length, 1)
  // 仅当几乎没有段落结构（整墙字）时才主动拆分
  const needsDenseSplit = merged.length <= 1 || (merged.length < 4 && avg > 220)

  let pieces: string[]
  if (needsDenseSplit) {
    pieces = []
    for (const block of merged) {
      if (block.length <= PACK_TARGET) {
        pieces.push(block)
        continue
      }
      const parts = splitDenseBlock(block)
      pieces.push(...(parts.length ? parts : [block]))
    }
  } else {
    pieces = merged
  }

  // 仅当段落明显过碎（如历史缓存「一句一段」）时才重新打包
  const avgAfter = pieces.reduce((n, p) => n + p.length, 0) / Math.max(pieces.length, 1)
  if (pieces.length >= 8 && avgAfter < 55) {
    pieces = packShortParagraphs(pieces, PACK_TARGET)
  }

  return pieces.filter(Boolean).join('\n\n').replace(/\n{3,}/g, '\n\n').trim()
}

/**
 * 将过密无段落文本拆成适中长度的段落。
 * 对话边界处优先断段；普通句号只作为「打包边界」，不单独成段。
 * @param block 一段较长的连续正文
 * @returns 拆分后的段落数组
 */
function splitDenseBlock(block: string): string[] {
  let s = block

  // 对话起止处硬换行，阅读节奏更清晰
  s = s.replace(/([。！？~～]|……|…)(?=[“「『])/g, '$1\n')
  s = s.replace(/([”」』])(?=[^“「『”」』\n\s])/g, '$1\n')

  // 句末标点作为软边界，供后续按目标长度打包
  s = s.replace(/([。！？][”」』'’]?)(?=[^\n])/g, '$1\u0001')

  const sentences = s
    .split(/\u0001|\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0)

  return packShortParagraphs(sentences, PACK_TARGET)
}

/**
 * 将过短的连续句子合并为接近目标字数的段落。
 * @param parts 句子或短段列表
 * @param target 目标段落大约字数
 * @returns 合并后的段落列表
 */
function packShortParagraphs(parts: string[], target: number): string[] {
  if (!parts.length) return []
  const out: string[] = []
  let buf = ''

  for (const part of parts) {
    if (!buf) {
      buf = part
      continue
    }
    // 已够长则落段；否则与下一段中文直接拼接（不加空格）
    if (buf.length >= target) {
      out.push(buf)
      buf = part
      continue
    }
    if (buf.length + part.length <= Math.floor(target * 1.6)) {
      buf += part
    } else {
      out.push(buf)
      buf = part
    }
  }
  if (buf) out.push(buf)
  return out
}
