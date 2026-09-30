import { useQuery, useQueryClient } from "@tanstack/react-query"
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
  const token = useAuthStore((s) => s.token)
  const setToken = useAuthStore((s) => s.setToken)
  return async (organizationId: string) => {
    const resp = await authApi.switchContext(organizationId, token!)
    setToken(resp.accessToken)
    await queryClient.invalidateQueries()
    return resp
  }
}
