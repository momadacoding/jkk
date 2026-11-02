import { useEffect, useRef, type ReactNode } from 'react'

/**
 * 出现时把自己滚进视野的块，给弹框里的错误与进度用。
 *
 * 按钮贴在框底以后，用户可以在滚动区的任何位置点主按钮；错误渲染在内容末尾，不滚过去的话
 * 它出在视野外，看起来像什么都没发生。被按钮条挡住的那一截靠 `.dialog` 的
 * `scroll-padding-bottom` 让开。
 *
 * 只在挂载时滚一次：重试时调用方先清掉错误，新的错误是重新挂载。
 */
export function Reveal({
  className,
  role,
  children,
}: {
  className: string
  role?: string
  children: ReactNode
}) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    // jsdom 没实现 scrollIntoView
    ref.current?.scrollIntoView?.({ block: 'nearest' })
  }, [])
  return (
    <div ref={ref} className={className} role={role}>
      {children}
    </div>
  )
}
