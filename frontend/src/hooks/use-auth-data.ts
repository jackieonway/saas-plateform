import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useRef } from "react"
import { useAuthStore } from "@/stores/auth"
import { authApi } from "@/api/auth"
import { asArray } from "@/lib/api"
import { sortMenuTree } from "@/lib/tree"
import type { MenuNode, PermissionsResp } from "@/api/types"

export function useProfileQuery() {
  const token = useAuthStore((s) => s.token)
  return useQuery({
    queryKey: ["profile"],
    queryFn: () => authApi.profile(),
    enabled: !!token,
    staleTime: 60_000,
    retry: 1,
  })
}

export function useMenusQuery(applicationId: string | null | undefined) {
  const token = useAuthStore((s) => s.token)
  return useQuery({
    queryKey: ["auth-menus", applicationId],
    queryFn: async () =>
      sortMenuTree(asArray<MenuNode>(await authApi.menus(applicationId!), "menus")),
    enabled: !!token && !!applicationId,
    staleTime: 30_000,
    retry: 1,
  })
}

export function usePermissionsQuery() {
  const token = useAuthStore((s) => s.token)
  return useQuery({
    queryKey: ["permissions"],
    queryFn: (): Promise<PermissionsResp> => authApi.permissions(),
    enabled: !!token,
    staleTime: 60_000,
    retry: 1,
  })
}

/** 组织上下文切换:POST context(旧 session token)→ 更新 token → 全量刷新 */
export function useSwitchContext() {
  const queryClient = useQueryClient()
  const setToken = useAuthStore((s) => s.setToken)
  const switching = useRef(false)
  return async (organizationId: string) => {
    if (switching.current) throw new Error("组织切换进行中，请稍候")
    switching.current = true
    try {
      const token = useAuthStore.getState().token
      if (!token) throw new Error("会话已失效，请重新登录")
      const resp = await authApi.switchContext(organizationId, token)
      if (useAuthStore.getState().token !== token) throw new Error("会话已变化，请重试")
      await queryClient.cancelQueries()
      queryClient.clear()
      setToken(resp.accessToken)
      return resp
    } finally {
      switching.current = false
    }
  }
}
