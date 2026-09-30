/** 跳转函数签名(兼容 react-router navigate 与侧栏 onNavigate 回调) */
export type NavigateFn = (to: string) => void

/**
 * 菜单外链统一跳转处理(侧边栏与命令面板共用, 避免两处复制):
 * - linkTarget === "SELF": 内部嵌入, 跳 /embed 路由由 iframe 承载
 * - 其余(含 BLANK/缺省): 新窗口打开, noopener noreferrer 防止 window.opener 泄漏
 *
 * @returns 是否按外链处理(调用方据此跳过常规路由跳转)
 */
export function openExternalLink(
  node: { linkUrl?: string | null; linkTarget?: string | null; title: string },
  navigate: NavigateFn
): boolean {
  const url = (node.linkUrl ?? "").trim()
  if (!url) return false
  if (node.linkTarget === "SELF") {
    navigate(
      `/embed?url=${encodeURIComponent(url)}&title=${encodeURIComponent(node.title)}`
    )
  } else {
    window.open(url, "_blank", "noopener,noreferrer")
  }
  return true
}
