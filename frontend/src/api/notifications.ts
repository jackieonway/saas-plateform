import { api } from "@/lib/api"
import type { NotificationRecord, NotificationSendBody, NotificationType } from "./types"
import { asPage } from "@/lib/api"

/** 通知偏好 (免打扰 + 静音类型), GET/PUT /api/notifications/prefs */
export interface NotificationPrefs {
  /** 免打扰开关:开启后时段内不弹通知提醒 */
  dndEnabled: boolean
  /** 免打扰开始 HH:MM (本地时间) */
  dndStart: string
  /** 免打扰结束 HH:MM (支持跨午夜,如 22:00 → 次日 07:00) */
  dndEnd: string
  /** 静音类型:这些类型不弹提醒,仅静默入列 */
  mutedTypes: NotificationType[]
}

export const notificationsApi = {
  list: (params: {
    filter?: "ALL" | "UNREAD"
    /** 可选类型筛选,与 filter 组合 (非法值后端 400) */
    type?: NotificationType
    /** 可选搜索词,title/content ILIKE,与 filter/type 组合 */
    q?: string
    page?: number
    size?: number
  }) => {
    const qs = new URLSearchParams()
    if (params.filter) qs.set("filter", params.filter)
    if (params.type) qs.set("type", params.type)
    const q = params.q?.trim()
    if (q) qs.set("q", q)
    qs.set("page", String(params.page ?? 1))
    qs.set("size", String(params.size ?? 10))
    return api
      .get<unknown>(`/api/notifications?${qs.toString()}`)
      .then((d) => asPage<NotificationRecord>(d))
  },
  unreadCount: () => api.get<{ count: number }>("/api/notifications/unread-count"),
  markRead: (id: string) => api.put<{ ok: boolean }>(`/api/notifications/${id}/read`),
  readAll: () => api.put<{ updated: number }>("/api/notifications/read-all"),
  /** 批量标记已读 → {"updated":N} (仅本人通知) */
  readBatch: (ids: string[]) =>
    api.put<{ updated: number }>("/api/notifications/read-batch", { ids }),
  remove: (id: string) => api.del<{ ok: boolean }>(`/api/notifications/${id}`),
  /** 批量删除 → {"deleted":N} (仅本人通知) */
  removeBatch: (ids: string[]) => api.del<{ deleted: number }>("/api/notifications", { ids }),
  send: (body: NotificationSendBody) => api.post<{ sent: number }>("/api/notifications", body),
  /** 读取通知偏好 */
  getPrefs: () => api.get<NotificationPrefs>("/api/notifications/prefs"),
  /** 保存通知偏好 (HH:MM 校验 + 类型白名单) */
  savePrefs: (body: NotificationPrefs) => api.put<NotificationPrefs>("/api/notifications/prefs", body),
}
