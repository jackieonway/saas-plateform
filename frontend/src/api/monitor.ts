import { api } from "@/lib/api"
import { authStore } from "@/stores/auth"
import type { HealthResp, MonitorOverview, MonitorSystemInfo } from "./types"

/** 租户洞察 (GET /api/monitor/tenant-insights) */
export interface TenantInsight {
  tenantId: string
  tenantName: string
  tenantCode: string
  status?: string
  users: number
  organizations: number
  roles: number
  logins7d: number
  activePlatformSessions: number
  /** 近 7 天每日登录数 (旧 → 今, 恒为 7 个元素) */
  loginsTrend: number[]
}

export interface TenantInsightsResp {
  insights: TenantInsight[]
}

export const monitorApi = {
  overview: () => api.get<MonitorOverview>("/api/monitor/overview"),
  health: () => api.get<HealthResp>("/api/health", { skipAuth: true, suppress401: true }),
  tenantInsights: () => api.get<TenantInsightsResp>("/api/monitor/tenant-insights"),
  /** 平台监控系统信息(服务进程/数据库/核心表, 权限 system:monitor, 后端契约 18-b) */
  system: () => api.get<MonitorSystemInfo>("/api/monitor/system"),
  /**
   * 导出租户洞察 CSV (后端返回带 BOM 的 text/csv 文件流, 权限 system:monitor)。
   * 与审计日志导出同款交互: Bearer fetch → blob → a[download],
   * 文件名优先取 content-disposition。
   */
  exportTenantInsights: async (): Promise<{ blob: Blob; filename: string }> => {
    const token = authStore.getState().token
    const res = await fetch("/api/monitor/tenant-insights/export", {
      headers: token ? { Authorization: `Bearer ${token}` } : undefined,
    })
    if (!res.ok) throw new Error(`导出失败 (HTTP ${res.status})`)
    const blob = await res.blob()
    const disposition = res.headers.get("content-disposition") ?? ""
    const filename =
      /filename="?([^";]+)"?/.exec(disposition)?.[1] ??
      `tenant-insights-${new Date().toISOString().slice(0, 10).replace(/-/g, "")}.csv`
    return { blob, filename }
  },
}
