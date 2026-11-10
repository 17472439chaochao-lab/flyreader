import { useState } from 'react'

export type SafeCoverProps = {
  /** 封面图片地址；为空直接渲染占位 */
  src?: string | null
  /** 书名：用于占位首字与 alt */
  title: string
  /** 尺寸/外观 class，默认 "cover" */
  className?: string
  /** 占位 class，默认 "cover placeholder" */
  placeholderClassName?: string
}

/**
 * 安全封面：img 带 loading="lazy"，加载失败时回落为书名首字占位，不留破图。
 * 注意：onError 只在图片真的加载失败时触发；空 src 直接走占位分支。
 */
export function SafeCover({
  src,
  title,
  className = 'cover',
  placeholderClassName = 'cover placeholder'
}: SafeCoverProps) {
  const [failed, setFailed] = useState(false)
  const first = (title || '?').slice(0, 1)
  if (!src || failed) {
    return <div className={placeholderClassName}>{first}</div>
  }
  return (
    <img
      className={className}
      src={src}
      alt=""
      loading="lazy"
      onError={() => setFailed(true)}
    />
  )
}
