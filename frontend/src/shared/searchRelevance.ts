/**
 * 规范化书名/作者，供搜索相关度比较使用。
 * @param raw 原始字符串，可为空
 * @returns 去空白、标点与常见版本文案后的小写文本
 */
export function normalizeSearchText(raw?: string | null): string {
  return (raw || '')
    .toLowerCase()
    .replace(/[\s\u3000]+/g, '')
    .replace(/[《》〈〉「」『』【】〔〕\[\]]/g, '')
    .replace(/[（）()·・．.。，,、：:；;！!？?""''"'|｜\-—_~～…]/g, '')
    .replace(
      /(精校版|无删减版?|完结版?|完本|全本|最新章节|全文阅读|txt|epub|mobi|azw3|vip)/gi,
      ''
    )
}

/**
 * 计算两字符串最长公共连续子串长度（适合短中文书名）。
 * @param a 字符串 A
 * @param b 字符串 B
 * @returns 最长公共子串长度；任一为空则为 0
 */
export function longestCommonSubstringLength(a: string, b: string): number {
  if (!a || !b) return 0
  const m = a.length
  const n = b.length
  let best = 0
  const row = new Array<number>(n + 1).fill(0)
  for (let i = 1; i <= m; i++) {
    let prev = 0
    for (let j = 1; j <= n; j++) {
      const tmp = row[j]
      if (a[i - 1] === b[j - 1]) {
        row[j] = prev + 1
        if (row[j] > best) best = row[j]
      } else {
        row[j] = 0
      }
      prev = tmp
    }
  }
  return best
}

/**
 * 计算书名相似度，范围 0～1。
 * 包含关系得分较高；否则用最长公共子串相对较短一侧长度。
 * @param keyword 搜索关键词
 * @param title 书名
 * @returns 相似度分数，0～1
 */
export function titleSimilarity(keyword: string, title: string): number {
  const key = normalizeSearchText(keyword)
  const name = normalizeSearchText(title)
  if (!key || !name) return 0
  if (name === key) return 1
  if (name.includes(key) || key.includes(name)) {
    const shorter = Math.min(key.length, name.length)
    const longer = Math.max(key.length, name.length)
    return shorter / longer
  }
  const lcs = longestCommonSubstringLength(key, name)
  const denom = Math.min(key.length, name.length)
  return denom ? lcs / denom : 0
}

/**
 * 判断搜索命中是否与关键词相关，过滤失败搜索时返回的首页/热榜垃圾结果。
 * 短关键词（≤4）：书名或作者须包含完整关键词。
 * 较长关键词：允许书名有较强连续重叠。
 * @param keyword 用户搜索词
 * @param book 含可选 name、author 的命中项
 * @returns 认为相关则 true
 */
export function isSearchHitRelevant(
  keyword: string,
  book: { name?: string; author?: string }
): boolean {
  const key = normalizeSearchText(keyword)
  if (!key) return true

  const name = normalizeSearchText(book.name)
  const author = normalizeSearchText(book.author)

  // 「遮天」「剑来」等短词：必须完整出现在书名（或作者）里，
  // 否则热门榜/乱序结果会大量混入（如「狮舞者」）。
  if (key.length <= 4) {
    if (name.includes(key)) return true
    if (author.includes(key)) return true
    return false
  }

  if (name) {
    if (name.includes(key)) return true
    // 查询包含完整书名（更长短语或带作者时）
    if (key.includes(name) && name.length >= 4) return true

    const lcs = longestCommonSubstringLength(key, name)
    const need = Math.max(3, Math.ceil(key.length * 0.6))
    if (lcs >= need) return true
  }

  if (author && author.length >= 2) {
    if (author.includes(key)) return true
    if (key.includes(author) && author.length >= 3) return true
  }

  return false
}

/**
 * 按相关度过滤搜索结果列表。
 * @param keyword 搜索关键词
 * @param books 原始命中列表
 * @returns 过滤后的相关命中；关键词为空或列表为空则原样返回
 */
export function filterRelevantSearchHits<T extends { name?: string; author?: string }>(
  keyword: string,
  books: T[]
): T[] {
  if (!keyword.trim() || !books.length) return books
  return books.filter((b) => isSearchHitRelevant(keyword, b))
}

/**
 * 换源候选的更严格匹配：要求像同一本小说，而非仅关键词相近。
 * @param original 原书（名称必填，作者可选）
 * @param hit 搜索命中项
 * @returns 可作为换源候选则 true
 */
export function isChangeSourceCandidate(
  original: { name: string; author?: string },
  hit: { name?: string; author?: string }
): boolean {
  const origName = normalizeSearchText(original.name)
  const hitName = normalizeSearchText(hit.name)
  if (!origName || !hitName) return false

  // 短书名：命中须包含原书名（或互相包含）
  if (origName.length <= 4) {
    return hitName.includes(origName) || origName.includes(hitName)
  }

  const sim = titleSimilarity(original.name, hit.name || '')
  if (sim >= 0.6) return true
  if (hitName.includes(origName) || origName.includes(hitName)) return true

  const origAuthor = normalizeSearchText(original.author)
  const hitAuthor = normalizeSearchText(hit.author)
  if (sim >= 0.4 && origAuthor && hitAuthor && origAuthor === hitAuthor) return true

  return false
}

/**
 * 过滤出适合换源的搜索命中。
 * @param original 原书身份信息
 * @param books 搜索结果列表
 * @returns 换源候选子集
 */
export function filterChangeSourceCandidates<T extends { name?: string; author?: string }>(
  original: { name: string; author?: string },
  books: T[]
): T[] {
  return books.filter((b) => isChangeSourceCandidate(original, b))
}

/**
 * 按 origin + bookUrl 对搜索书籍去重。
 * @param books 可能含重复项的列表
 * @returns 去重后的列表（保留首次出现）
 */
export function dedupeSearchBooks<T extends { origin: string; bookUrl: string }>(books: T[]): T[] {
  const seen = new Set<string>()
  const out: T[] = []
  for (const b of books) {
    const id = `${b.origin}::${b.bookUrl}`
    if (seen.has(id)) continue
    seen.add(id)
    out.push(b)
  }
  return out
}

/**
 * 根据某书源一次搜索的原始结果与过滤结果，打出 0～100 匹配质量分。
 * 空结果或全是垃圾结果时分数偏低，使嘈杂书源优先级下降。
 * @param keyword 搜索关键词
 * @param raw 过滤前的原始命中
 * @param filtered 相关度过滤后的命中
 * @returns 0～100 的整数匹配分
 */
export function scoreSearchMatchSample(
  keyword: string,
  raw: { name?: string }[],
  filtered: { name?: string }[]
): number {
  if (!raw.length) return 0
  if (!filtered.length) return 8
  const sims = filtered.map((b) => titleSimilarity(keyword, b.name || ''))
  const best = Math.max(...sims)
  const avg = sims.reduce((s, n) => s + n, 0) / sims.length
  const precision = filtered.length / raw.length
  return Math.round(100 * (0.5 * best + 0.3 * avg + 0.2 * precision))
}

/**
 * 用指数移动平均更新 flyMatchScore。
 * @param prev 历史匹配分，可为空
 * @param sample 本次采样分
 * @returns 钳制在 0～100 的新分数
 */
export function blendMatchScore(prev: number | undefined, sample: number): number {
  const next = Math.max(0, Math.min(100, Math.round(sample)))
  if (prev == null || !Number.isFinite(prev)) return next
  return Math.round(Math.max(0, Math.min(100, prev)) * 0.65 + next * 0.35)
}
