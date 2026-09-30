import { api } from "@/lib/api"
import type { AuditLogRecord } from "./types"

export const auditApi = {
  /** tenantId: 缺省/ALL=不过滤; "none"=仅平台级行; uuid=指定租户 */
  list: (params: { page?: number; size?: number; keyword?: string; action?: string; tenantId?: string }) => {
    const qs = new URLSearchParams()
    if (params.page) qs.set("page", String(params.page))
    if (params.size) qs.set("size", String(params.size))
    if (params.keyword) qs.set("keyword", params.keyword)
    if (params.action && params.action !== "ALL") qs.set("action", params.action)
    if (params.tenantId && params.tenantId !== "ALL") qs.set("tenant_id", params.tenantId)
    const q = qs.toString()
    return api.get<unknown>(`/api/audit-logs${q ? `?${q}` : ""}`)
  },
}
