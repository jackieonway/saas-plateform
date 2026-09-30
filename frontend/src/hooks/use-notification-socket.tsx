import * as React from "react"
import { io, type Socket } from "socket.io-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { CircleCheck, Info, ShieldAlert, TriangleAlert } from "lucide-react"
import { toast } from "sonner"
import { useAuthStore } from "@/stores/auth"
import type { NotificationType } from "@/api/types"
import { notificationsApi, type NotificationPrefs } from "@/api/notifications"
import { cn } from "@/lib/utils"

/** pg_notify new_notification 通道推送的载荷 (后端 notify() 构造) */
export interface RealtimeNotification {
  userId?: string
  id: string
  type?: NotificationType
  title?: string
  content?: string | null
  link?: string | null
  createdAt?: string
}

/** toast 四色:INFO 默认 / SUCCESS 成功 / WARNING 警告 / SECURITY 危险 */
const TOAST_META: Record<NotificationType, { icon: typeof Info; cls: string }> = {
  INFO: { icon: Info, cls: "text-primary" },
  SUCCESS: { icon: CircleCheck, cls: "text-emerald-600 dark:text-emerald-400" },
  WARNING: { icon: TriangleAlert, cls: "text-amber-600 dark:text-amber-400" },
  SECURITY: { icon: ShieldAlert, cls: "text-destructive" },
}

/** toast 节流:10 秒内相同标题去重 + 滑动窗口内最多 3 条(防广播风暴连弹) */
const TOAST_DEDUP_MS = 10_000
const TOAST_MAX_PER_WINDOW = 3

/** "HH:MM" → 当日分钟数(0-1439);非法返回 NaN */
function parseHM(hm: string | undefined | null): number {
  const m = /^(\d{1,2}):(\d{2})$/.exec((hm ?? "").trim())
  if (!m) return NaN
  const h = Number(m[1])
  const min = Number(m[2])
  if (h > 23 || min > 59) return NaN
  return h * 60 + min
}

/**
 * 当前本地时间是否落在免打扰区间内。
 * start < end → 常规区间 [start, end);
 * start > end → 跨午夜 [start, 24:00) ∪ [00:00, end);
 * start == end → 区间并集为全天。
 */
export function inDndWindow(start: string, end: string, now = new Date()): boolean {
  const s = parseHM(start)
  const e = parseHM(end)
  if (Number.isNaN(s) || Number.isNaN(e)) return false
  const cur = now.getHours() * 60 + now.getMinutes()
  if (s === e) return true
  if (s < e) return cur >= s && cur < e
  return cur >= s || cur < e
}

/**
 * 实时通知 WebSocket hook —— 整个应用只挂载一次 (TopBar)。
 *
 * - 仅在已登录 (zustand auth store 有 token) 时连接 io('/?XTransformPort=3003');
 * - 连接后 emit authenticate {token} → 监听 authenticated / notification;
 * - notification 处理顺序:
 *   ① invalidate ["notifications"] (铃铛计数/通知中心自动刷新,永远执行);
 *   ② 偏好抑制: 类型 ∈ mutedTypes → 不弹;免打扰开启且当前时间落在区间内 → 不弹;
 *   ③ toast 节流: 10s 内相同 title 只弹一次;10s 滑动窗口内最多 3 条;
 * - 偏好 (["notification-prefs"] 查询缓存) 保存后即时生效;
 * - connect_error 静默处理, 降级为现有 60s 轮询兜底;
 * - token 变化 / 组件卸载 / 登出时 socket.disconnect()。
 */
export function useNotificationSocket(): {
  connected: boolean
  /** 最近一次收到实时通知的时间戳 (供铃铛做一次性摆动动画) */
  lastNotificationAt: number
  /** 通知偏好 (免打扰 + 静音类型), 供铃铛等处共享缓存使用 */
  prefs: NotificationPrefs | undefined
} {
  const token = useAuthStore((s) => s.token)
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [connected, setConnected] = React.useState(false)
  const [lastNotificationAt, setLastNotificationAt] = React.useState(0)

  /* 通知偏好:与通知设置对话框同 key, 缓存共享无重复请求 */
  const prefsQuery = useQuery({
    queryKey: ["notification-prefs"],
    queryFn: () => notificationsApi.getPrefs(),
    enabled: !!token,
    staleTime: 30_000,
    retry: 1,
  })

  /* toast 节流状态(ref 持久, 重连后仍生效): title → 上次弹出时间 / 弹出时间戳滑动窗口 */
  const toastTitleAt = React.useRef(new Map<string, number>())
  const toastWindow = React.useRef<number[]>([])

  React.useEffect(() => {
    if (!token) return

    // 绝不写死端口, 必须经 XTransformPort 转发 (Caddy 网关 / vite dev 代理同构)
    const socket: Socket = io("/?XTransformPort=3003", {
      transports: ["websocket", "polling"],
      reconnection: true,
      reconnectionAttempts: 8,
      reconnectionDelay: 2000,
      timeout: 10000,
    })

    socket.on("connect", () => {
      socket.emit("authenticate", { token })
    })
    socket.on("authenticated", (d: { userId?: string }) => {
      console.debug("[notification-ws] 实时通知已连接", d)
      setConnected(true)
    })
    socket.on("unauthorized", (d: { message?: string }) => {
      // 令牌被拒 (过期/类型不对): 停止重连, 交给 60s 轮询兜底, 待重新登录后重建
      console.debug("[notification-ws] 认证被拒", d)
      setConnected(false)
      socket.io.reconnection(false)
    })
    socket.on("disconnect", () => setConnected(false))
    socket.on("connect_error", () => {
      // 静默降级: 保持现有 60s 轮询兜底
      setConnected(false)
    })

    socket.on("notification", (n: RealtimeNotification) => {
      const type = normalizeType(n.type)
      const meta = TOAST_META[type]
      const title = n.title?.trim() || "新通知"
      const link = n.link?.trim() || ""
      const description = n.content?.trim() || undefined

      // ① 铃铛计数 / 通知中心列表自动刷新(无论是否弹 toast 都执行)
      void queryClient.invalidateQueries({ queryKey: ["notifications"] })
      setLastNotificationAt(Date.now())

      /* ② 偏好抑制:从缓存实时读取(保存后 invalidate 即时生效) */
      const prefs = queryClient.getQueryData<NotificationPrefs>(["notification-prefs"])
      if (prefs?.mutedTypes?.includes(type)) {
        console.debug(`[notification-ws] 类型 ${type} 已静音,不弹提醒:`, title)
        return
      }
      if (prefs?.dndEnabled && inDndWindow(prefs.dndStart, prefs.dndEnd)) {
        console.debug(
          `[notification-ws] 免打扰中(${prefs.dndStart}-${prefs.dndEnd}),不弹提醒:`,
          title
        )
        return
      }

      /* ---- ③ toast 节流 ---- */
      const now = Date.now()
      // 过期清理: 相同标题去重表 + 滑动窗口
      for (const [t, at] of toastTitleAt.current) {
        if (now - at >= TOAST_DEDUP_MS) toastTitleAt.current.delete(t)
      }
      toastWindow.current = toastWindow.current.filter((at) => now - at < TOAST_DEDUP_MS)

      const dupTitle = toastTitleAt.current.has(title)
      const overLimit = toastWindow.current.length >= TOAST_MAX_PER_WINDOW
      if (dupTitle || overLimit) {
        // 10s 内同标题已弹过 或 窗口内已达 3 条: 不再弹 toast
        return
      }
      toastTitleAt.current.set(title, now)
      toastWindow.current.push(now)

      const go = link ? () => navigate(link) : undefined
      const Icon = meta.icon
      toast(title, {
        description,
        icon: <Icon className={cn("size-4", meta.cls)} />,
        duration: 6000,
        // sonner: 点击 toast 上的「查看」按钮跳转 (有 link 时)
        action: go ? { label: "查看", onClick: go } : undefined,
      })
    })

    return () => {
      socket.removeAllListeners()
      socket.disconnect()
      setConnected(false)
    }
  }, [token, queryClient, navigate])

  return { connected, lastNotificationAt, prefs: prefsQuery.data }
}

function normalizeType(t: unknown): NotificationType {
  return t === "SUCCESS" || t === "WARNING" || t === "SECURITY" || t === "INFO" ? t : "INFO"
}
