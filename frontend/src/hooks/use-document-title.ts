import { useEffect } from "react"
import { ROUTE_TITLES } from "@/lib/routes"

const APP_NAME = "Dante Cloud"

/** 路径 → 页面名(未匹配时兜底「工作台」) */
export function pageTitleFor(pathname: string): string {
  return ROUTE_TITLES[pathname] ?? "工作台"
}

/**
 * 设置 document.title 为「<页面名> · Dante Cloud」
 * pageName 为 null 时跳过(由页面自行管理标题, 如 /embed 随菜单标题动态变化)
 */
export function useDocumentTitle(pageName: string | null) {
  useEffect(() => {
    if (pageName === null) return
    document.title = pageName ? `${pageName} · ${APP_NAME}` : APP_NAME
  }, [pageName])
}
