import * as React from "react"

/**
 * 通知中心键盘快捷键(仅桌面 md+ 生效):
 * - J / ArrowDown:焦点行下移;K / ArrowUp:上移(焦点行 ring 高亮 + scrollIntoView nearest);
 * - Space:切换焦点行的选中(复选框状态);
 * - X:将当前选中的未读行批量已读(复用 readBatch 逻辑);
 * - Esc:清空选择 + 焦点复位;
 * - ? (Shift + /):打开键盘快捷键帮助(开关状态由页面自管, 经 onOpenHelp 回调触发);
 * - 输入控件聚焦(INPUT/TEXTAREA/SELECT/contentEditable)与对话框打开时不触发。
 */
export interface NotificationHotkeyHandlers {
  /** Space:切换第 index 行(0 起)的选中态 */
  onToggleSelect: (index: number) => void
  /** X:批量已读当前选中的未读行(参数为焦点行下标, 供无选中时兜底单条已读) */
  onMarkSelectedRead: (focusIndex: number) => void
  /** Esc:清空选择(焦点由 hook 自行复位) */
  onClearSelection: () => void
  /** ?:打开键盘快捷键帮助(Dialog 打开期间 enabled 为 false, 不会重复触发) */
  onOpenHelp: () => void
}

const MD_QUERY = "(min-width: 768px)"

export function useNotificationHotkeys({
  enabled,
  count,
  handlers,
}: {
  /** 列表就绪且无对话框遮挡时为 true */
  enabled: boolean
  /** 当前行数 */
  count: number
  handlers: NotificationHotkeyHandlers
}): {
  /** 当前焦点行下标(-1 表示无焦点) */
  focusIndex: number
  /** 焦点复位(-1) */
  resetFocus: () => void
  /** 是否处于桌面宽度(md+, 提示条按此显隐) */
  isDesktop: boolean
} {
  const [focusIndex, setFocusIndex] = React.useState(-1)
  const [isDesktop, setIsDesktop] = React.useState(() =>
    typeof window === "undefined" ? false : window.matchMedia(MD_QUERY).matches
  )

  /* 桌面断点监听(快捷键与提示条均按 md+ 生效) */
  React.useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia === "undefined") return
    const mql = window.matchMedia(MD_QUERY)
    const onChange = (e: MediaQueryListEvent) => setIsDesktop(e.matches)
    mql.addEventListener("change", onChange)
    return () => mql.removeEventListener("change", onChange)
  }, [])

  /* 以 ref 读取最新值,避免闭包过期 */
  const enabledRef = React.useRef(enabled)
  const countRef = React.useRef(count)
  const focusRef = React.useRef(focusIndex)
  const handlersRef = React.useRef(handlers)
  const desktopRef = React.useRef(isDesktop)
  enabledRef.current = enabled
  countRef.current = count
  focusRef.current = focusIndex
  handlersRef.current = handlers
  desktopRef.current = isDesktop

  const resetFocus = React.useCallback(() => setFocusIndex(-1), [])

  /* 行数收缩时把越界焦点拉回(记录集变化由页面侧 resetFocus 处理) */
  React.useEffect(() => {
    setFocusIndex((i) => (i >= count ? -1 : i))
  }, [count])

  React.useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!enabledRef.current) return
      if (e.metaKey || e.ctrlKey || e.altKey) return
      if (!desktopRef.current) return
      // 输入控件聚焦时不接管按键
      const t = e.target as HTMLElement | null
      if (
        t &&
        (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable)
      ) {
        return
      }

      // ?:帮助与列表内容无关, 空列表/筛选零结果时也可打开(置于 count 早退之前)
      if (e.key === "?") {
        e.preventDefault()
        handlersRef.current.onOpenHelp()
        return
      }
      if (countRef.current === 0) return

      const key = e.key.toLowerCase()
      switch (key) {
        case "j":
        case "arrowdown":
          e.preventDefault()
          setFocusIndex((i) => Math.min(countRef.current - 1, i + 1))
          break
        case "k":
        case "arrowup":
          e.preventDefault()
          setFocusIndex((i) => Math.max(0, i - 1))
          break
        case " ":
        case "spacebar":
          e.preventDefault()
          if (focusRef.current >= 0) handlersRef.current.onToggleSelect(focusRef.current)
          break
        case "x":
          e.preventDefault()
          handlersRef.current.onMarkSelectedRead(focusRef.current)
          break
        case "escape":
          e.preventDefault()
          handlersRef.current.onClearSelection()
          setFocusIndex(-1)
          break
        default:
          break
      }
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [])

  return { focusIndex, resetFocus, isDesktop }
}
