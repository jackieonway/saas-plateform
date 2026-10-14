import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useAuthStore } from "@/stores/auth"
import { authApi } from "@/api/auth"
import { asArray } from "@/lib/api"
import { sortMenuTree } from "@/lib/tree"
import type { MenuNode, PermissionsResp } from "@/api/types"

export function useProfileQuery() {
  const token = useAuthStore((s) => s.token)
  const epoch = useAuthStore((s) => s.contextEpoch)
  const switching = useAuthStore((s) => s.contextSwitching)
  return useQuery({
    queryKey: ["profile", epoch],
    queryFn: ({ signal }) => authApi.profile(undefined, signal),
    enabled: !!token && !switching,
    staleTime: 20_000,
    refetchInterval: 20_000,
    retry: 1,
  })
}

export function useMenusQuery(applicationId: string | null | undefined) {
  const token = useAuthStore((s) => s.token)
  const epoch = useAuthStore((s) => s.contextEpoch)
  const switching = useAuthStore((s) => s.contextSwitching)
  return useQuery({
    queryKey: ["auth-menus", applicationId, epoch],
    queryFn: async ({ signal }) =>
      sortMenuTree(asArray<MenuNode>(await authApi.menus(applicationId!, undefined, signal), "menus")),
    enabled: !!token && !!applicationId && !switching,
    staleTime: 20_000,
    refetchInterval: 20_000,
    retry: 1,
  })
}

export function usePermissionsQuery() {
  const token = useAuthStore((s) => s.token)
  const epoch = useAuthStore((s) => s.contextEpoch)
  const switching = useAuthStore((s) => s.contextSwitching)
  return useQuery({
    queryKey: ["permissions", epoch],
    queryFn: ({ signal }): Promise<PermissionsResp> => authApi.permissions(undefined, signal),
    enabled: !!token && !switching,
    staleTime: 20_000,
    refetchInterval: 20_000,
    retry: 1,
  })
}

/** 组织上下文切换:POST context(旧 session token)→ 更新 token → 全量刷新 */
export function useSwitchContext() {
  const queryClient = useQueryClient()
  return async (organizationId: string) => {
    const state = useAuthStore.getState()
    if (state.contextSwitching) throw new Error("组织切换进行中，请稍候")
    const token = state.token
    if (!token) throw new Error("会话已失效，请重新登录")
    state.beginContextSwitch()
    const switchEpoch = useAuthStore.getState().contextEpoch
    try {
      await queryClient.cancelQueries()
      queryClient.clear()
      const resp = await authApi.switchContext(organizationId, token)
      if (useAuthStore.getState().contextEpoch !== switchEpoch) throw new Error("会话已变化，请重试")
      useAuthStore.getState().setSession(resp.accessToken, resp.refreshToken)
      return resp
    } finally {
      if (useAuthStore.getState().contextEpoch === switchEpoch) useAuthStore.getState().endContextSwitch()
    }
  }
}
