import { api } from "@/lib/api"

/** 全局实体搜索结果(GET /api/search),无权限的分区为空数组 */
export interface GlobalSearchResult {
  users: {
    id: string
    username: string
    realName: string
    organizationName: string | null
    status: string
  }[]
  tenants: {
    id: string
    name: string
    code: string
    status: string
  }[]
  organizations: {
    id: string
    name: string
    code: string
    path: string
  }[]
  roles: {
    id: string
    name: string
    code: string
    organizationName: string | null
  }[]
}

export const EMPTY_SEARCH_RESULT: GlobalSearchResult = {
  users: [],
  tenants: [],
  organizations: [],
  roles: [],
}

/** 跨实体搜索用户/租户/组织/角色(每类默认最多 5 条) */
export function searchEntities(q: string, limit = 5): Promise<GlobalSearchResult> {
  return api.get<GlobalSearchResult>(
    `/api/search?q=${encodeURIComponent(q)}&limit=${limit}`
  )
}
