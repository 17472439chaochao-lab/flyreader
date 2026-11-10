import { describe, expect, it } from 'vitest'
import { renderMarkdownHtml, sanitizeHref } from './renderMarkdown'

describe('renderMarkdown XSS 回归', () => {
  it('<script> 标签应被清除', () => {
    const html = renderMarkdownHtml('你好\n\n<script>alert(1)</script>\n\n正文')
    expect(html).not.toContain('<script')
    expect(html).not.toContain('alert(1)')
    expect(html).toContain('正文')
  })

  it('<svg onload> 应被清除', () => {
    const html = renderMarkdownHtml('<svg onload="alert(1)"><circle /></svg>')
    expect(html).not.toContain('<svg')
    expect(html).not.toContain('onload')
  })

  it('<img onerror> 应被清除（原始 HTML 不进渲染）', () => {
    const html = renderMarkdownHtml('<img src="x" onerror="alert(1)">')
    expect(html).not.toContain('onerror')
    expect(html).not.toContain('alert(1)')
  })

  it('javascript: 链接应被剥离，只保留链接文本', () => {
    const html = renderMarkdownHtml('[点我](javascript:alert(1))')
    expect(html).not.toContain('javascript:')
    expect(html).toContain('点我')
  })

  it('// 开头的协议相对 URL 与 data: 应被拒绝', () => {
    expect(sanitizeHref('//evil.example/x')).toBe('')
    expect(sanitizeHref('data:text/html,<script>alert(1)</script>')).toBe('')
  })

  it('https 链接与 # 锚点放行，mailto 做白名单校验', () => {
    expect(sanitizeHref('https://example.com/a')).toBe('https://example.com/a')
    expect(sanitizeHref('#md-h-0')).toBe('#md-h-0')
    expect(sanitizeHref('mailto:a@example.com')).toBe('mailto:a@example.com')
    expect(sanitizeHref('mailto:not-an-email')).toBe('')
    expect(sanitizeHref('mailto:a@b.com" onclick="alert(1)')).toBe('')
  })

  it('style/form/iframe 标签应被清除', () => {
    const html = renderMarkdownHtml('<form action="/x"><input></form><iframe src="https://example.com"></iframe>')
    expect(html).not.toContain('<form')
    expect(html).not.toContain('<iframe')
  })
})
