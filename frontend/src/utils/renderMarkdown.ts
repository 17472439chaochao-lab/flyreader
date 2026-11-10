import { Marked } from 'marked'
import DOMPurify from 'dompurify'

// 显式白名单：只放行渲染 Markdown 正文必需的标签与属性，默认 html profile 之外再收紧
const ALLOWED_TAGS = [
  'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
  'p', 'br', 'hr', 'blockquote', 'pre', 'code',
  'ul', 'ol', 'li', 'dl', 'dt', 'dd',
  'a', 'img', 'strong', 'em', 'del', 's', 'ins',
  'table', 'thead', 'tbody', 'tfoot', 'tr', 'th', 'td',
  'span', 'div', 'sub', 'sup', 'kbd', 'mark', 'abbr'
]
const ALLOWED_ATTR = ['href', 'title', 'alt', 'src', 'id', 'target', 'rel', 'loading', 'colspan', 'rowspan', 'align']
// 明确禁用的标签（即使白名单漏网也不放行）
const FORBID_TAGS = ['style', 'form', 'iframe', 'object', 'embed', 'svg', 'math', 'script', 'link', 'meta', 'base', 'input', 'button', 'textarea', 'select', 'video', 'audio', 'source', 'canvas']
// 明确禁用的属性：全部 on* 事件属性与内联 style（on* 用正则兜底，见 FORBID_ATTR）
const FORBID_ATTR = ['style', 'onabort', 'onblur', 'oncancel', 'oncanplay', 'onchange', 'onclick', 'onclose', 'oncontextmenu', 'oncuechange', 'ondblclick', 'ondrag', 'ondragend', 'ondragenter', 'ondragleave', 'ondragover', 'ondragstart', 'ondrop', 'ondurationchange', 'onemptied', 'onended', 'onerror', 'onfocus', 'oninput', 'oninvalid', 'onkeydown', 'onkeypress', 'onkeyup', 'onload', 'onloadeddata', 'onloadedmetadata', 'onloadstart', 'onmousedown', 'onmouseenter', 'onmouseleave', 'onmousemove', 'onmouseout', 'onmouseover', 'onmouseup', 'onmousewheel', 'onpause', 'onplay', 'onplaying', 'onprogress', 'onratechange', 'onreset', 'onresize', 'onscroll', 'onseeked', 'onseeking', 'onselect', 'onshow', 'onstalled', 'onsubmit', 'onsuspend', 'ontimeupdate', 'ontoggle', 'onvolumechange', 'onwaiting', 'onwheel']

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
    // walkTokens 丢弃 html token：原始 HTML 不进渲染，从源头堵住 <script>/<svg onload> 等注入
    walkTokens(token) {
      if (token.type === 'html') {
        token.text = ''
        token.raw = ''
      }
    },
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
    ALLOWED_TAGS,
    ALLOWED_ATTR,
    FORBID_TAGS,
    FORBID_ATTR
  })
}

/**
 * 链接白名单：只放行 https:；本地相对路径与 # 锚点除外；
 * mailto: 做长度与字符白名单校验；拒绝 // 开头的协议相对 URL 与 data:。
 */
export function sanitizeHref(href: string): string {
  const h = href.trim()
  if (!h) return ''
  // 拒绝协议相对 URL（//evil.com）与 data:（可藏 javascript: 载荷）
  if (h.startsWith('//') || /^data:/i.test(h)) return ''
  if (/^https:\/\//i.test(h)) return h
  if (h.startsWith('#') || h.startsWith('./') || h.startsWith('../') || h.startsWith('/')) return h
  if (/^mailto:/i.test(h)) {
    // mailto: 只允许常规邮箱字符，长度上限 254，且不含空白与引号/尖括号
    const addr = h.slice('mailto:'.length)
    if (!addr || addr.length > 254) return ''
    if (!/^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$/.test(addr)) return ''
    return h
  }
  // 无协议头的相对路径（如 foo/bar.md）放行；其余协议（http:/javascript:/vbscript: 等）一律拒绝
  if (!/^[a-z][a-z0-9+.-]*:/i.test(h)) return h
  return ''
}

function escapeAttr(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}
