import * as React from "react"
import { useQueries } from "@tanstack/react-query"
import { useAuthStore } from "@/stores/auth"
import { authApi } from "@/api/auth"
import { asArray } from "@/lib/api"
import { sortMenuTree } from "@/lib/tree"
import type { MenuNode } from "@/api/types"

/** 个人页面白名单:任何登录用户恒可访问(不在任何应用菜单树内,恒放行) */
const ALWAYS_ALLOWED_PATHS: readonly string[] = [
  "/dashboard",
  "/profile",
  "/system/notifications",
]

/** 递归收集菜单树中可跳转节点(type !== BUTTON 且 path 非空)的 path */
function collectPaths(nodes: MenuNode[], out: Set<string>): void {
  for (const n of nodes) {
    if (n.type !== "BUTTON" && n.path) out.add(n.path)
    if (n.children?.length) collectPaths(n.children, out)
  }
}

/**
 * 已授权路径集合(展示层权限过滤共用):
 * - 数据源:profile.applications 全部已授权应用 → useQueries 逐应用拉菜单树
 *   (queryKey ["auth-menus", app.id] 与侧栏/命令面板全局缓存共享)
 * - flatten 全部菜单树,收集可跳转 path 到 Set(与 CommandPalette flattenMenus 同口径)
 * - 白名单恒放行:/dashboard、/profile、/system/notifications
 * - ready = 所有应用查询 isSuccess 或 isError;isError 的应用按空集计(不放行其路径)
 */
export function useAuthorizedPaths(): { allowedPaths: Set<string>; ready: boolean } {
  const token = useAuthStore((s) => s.token)
  const profile = useAuthStore((s) => s.profile)
  const apps = profile?.applications ?? []

  const menusQueries = useQueries({
    queries: apps.map((app) => ({
      queryKey: ["auth-menus", app.id],
      queryFn: async () =>
        sortMenuTree(asArray<MenuNode>(await authApi.menus(app.id), "menus")),
      staleTime: 60_000,
      retry: 1,
      enabled: !!token,
    })),
  })

  /* 数据版本签名(dataUpdatedAt 随每次写入新数据变化):据此重建 Set,保持引用稳定 */
  const dataVersion = menusQueries.map((q) => q.dataUpdatedAt).join("|")
  const allowedPaths = React.useMemo(() => {
    const paths = new Set<string>(ALWAYS_ALLOWED_PATHS)
    for (const q of menusQueries) {
      if (q.data) collectPaths(q.data, paths)
    }
    return paths
    // eslint-disable-next-line react-hooks/exhaustive-deps -- menusQueries 内容由 dataVersion 唯一决定
  }, [dataVersion])

  /* profile 未加载时应用清单未知,视为未就绪;未登录(查询禁用)时恒 pending 亦未就绪 */
  const ready = profile !== null && menusQueries.every((q) => q.isSuccess || q.isError)

  return { allowedPaths, ready }
}
