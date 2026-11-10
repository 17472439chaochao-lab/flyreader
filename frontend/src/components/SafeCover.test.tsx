import { describe, expect, it } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { SafeCover } from './SafeCover'

describe('SafeCover', () => {
  it('图片加载失败时显示首字占位而不是破图', () => {
    render(<SafeCover src="https://example.com/broken.jpg" title="剑来" />)
    // 空 alt 的 img 在无障碍树里是 presentational，getByRole 找不到：直接取 DOM 节点
    const img = document.querySelector('img') as HTMLImageElement
    expect(img).toBeTruthy()
    // jsdom 下 img 不会真的发请求：用 fireEvent.error 模拟加载失败
    fireEvent.error(img)
    expect(screen.getByText('剑')).toBeTruthy()
    expect(document.querySelector('img')).toBeNull()
  })

  it('无封面地址时直接渲染首字占位', () => {
    render(<SafeCover title="遮天" />)
    expect(screen.getByText('遮')).toBeTruthy()
  })

  it('加载成功时保留 img（不触发占位）', () => {
    render(<SafeCover src="https://example.com/ok.jpg" title="剑来" />)
    expect(document.querySelector('img')).toBeTruthy()
  })
})
