import { Marked } from 'marked'
import DOMPurify from 'dompurify'

/**
 * 将 Markdown 转为经 DOMPurify 消毒的 HTML，并为每个标题写入 md-h-N 锚点。
 * @param markdown - 原始 Markdown 文本
 * @returns 安全 HTML 字符串
 */
export function renderMarkdownHtml(markdown: string): string {
  const src = (markdown || '').trim()
  if (!src) return '<p>（正文为空）</p>'

  let headingIndex = 0
  const marked = new Marked()
  marked.use({
    gfm: true,
    breaks: false,
    renderer: {
      heading({ tokens, depth }) {
        const body = this.parser.parseInline(tokens)
        const id = `md-h-${headingIndex++}`
        return `<h${depth} id="${id}">${body}</h${depth}>\n`
      },
      link({ href, title, tokens }) {
        const text = this.parser.parseInline(tokens)
        const t = title ? ` title="${escapeAttr(title)}"` : ''
        const safeHref = sanitizeHref(href || '')
        if (!safeHref) return text
        return `<a href="${escapeAttr(safeHref)}"${t} rel="noopener noreferrer">${text}</a>`
      },
      image({ href, title, text }) {
        const safeHref = sanitizeHref(href || '')
        if (!safeHref) return escapeHtml(text || '')
        const t = title ? ` title="${escapeAttr(title)}"` : ''
        const alt = escapeAttr(text || '')
        return `<img src="${escapeAttr(safeHref)}" alt="${alt}"${t} loading="lazy" />`
      }
    }
  })

  const dirty = marked.parse(src, { async: false }) as string
  return DOMPurify.sanitize(dirty, {
    USE_PROFILES: { html: true },
    ADD_ATTR: ['target', 'rel', 'loading', 'id']
  })
}

function sanitizeHref(href: string): string {
  const h = href.trim()
  if (!h) return ''
  if (/^https?:\/\//i.test(h) || /^mailto:/i.test(h)) return h
  if (h.startsWith('#') || h.startsWith('./') || h.startsWith('../') || !/^[a-z][a-z0-9+.-]*:/i.test(h)) {
    // 允许相对路径与页内锚点；拒绝 javascript: 等危险协议
    if (/^javascript:/i.test(h) || /^data:/i.test(h) || /^vbscript:/i.test(h)) return ''
    return h
  }
  return ''
}

function escapeAttr(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}
