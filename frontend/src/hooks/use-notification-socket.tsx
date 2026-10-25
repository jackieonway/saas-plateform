import * as React from "react"
import { io, type Socket } from "socket.io-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "react-router-dom"
import { CircleCheck, Info, ShieldAlert, TriangleAlert } from "lucide-react"
import { toast } from "sonner"
import type { NotificationSocketContext, NotificationType } from "@/api/types"
import { notificationsApi, type NotificationPrefs } from "@/api/notifications"
import { cn } from "@/lib/utils"
import { getNotificationContext, notificationRequestOptions, useNotificationContext, type NotificationContext } from "./use-notification-context"
import { isNotificationSocketContext, isRealtimeNotification, notificationLink } from "@/lib/notification-protocol"
export type { RealtimeNotification } from "@/api/types"

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
 * /socket.io/ 与网关同构；连接绑定捕获的 session、epoch 与完整 profile 身份。
 * auth store 开始切换时 signal 同步中止，立即断开旧连接；每个回调再验捕获上下文。
 * authenticated/reconnect 从数据库补拉列表、count、个人偏好，不补弹历史 Toast。
 */
export function useNotificationSocket(): {
  connected: boolean
  /** 最近一次收到实时通知的时间戳 (供铃铛做一次性摆动动画) */
  lastNotificationAt: number
  /** 通知偏好 (免打扰 + 静音类型), 供铃铛等处共享缓存使用 */
  prefs: NotificationPrefs | undefined
} {
  const scope = useNotificationContext()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const navigateRef = React.useRef(navigate)
  navigateRef.current = navigate
  const [state, setState] = React.useState<{ scope: NotificationContext | null; connected: boolean; lastNotificationAt: number }>(
    { scope: null, connected: false, lastNotificationAt: 0 })

  /* 通知偏好:与通知设置对话框同 key, 缓存共享无重复请求 */
  const prefsQuery = useQuery({
    queryKey: scope ? [...scope.key, "prefs"] : ["notifications", "inactive", "prefs"],
    queryFn: ({ signal }) => notificationsApi.getPrefs(notificationRequestOptions(scope!, signal)),
    enabled: !!scope && scope.current(),
    staleTime: 30_000,
    retry: 1,
  })

  React.useEffect(() => {
    if (!scope || !scope.current()) return
    const captured = scope
    let stopped = false
    let authenticated: NotificationSocketContext | null = null
    let expiryTimer: ReturnType<typeof setTimeout> | null = null
    let authTimer: ReturnType<typeof setTimeout> | null = null
    const seenIds = new Set<string>()
    const toastTitleAt = new Map<string, number>()
    let toastWindow: number[] = []
    const toastIds = new Set<string | number>()
    setState({ scope: captured, connected: false, lastNotificationAt: 0 })
    const socket: Socket = io(window.location.origin, {
      path: import.meta.env.VITE_NOTIFICATION_SOCKET_PATH || "/socket.io/",
      transports: ["websocket", "polling"],
      reconnection: true,
      reconnectionAttempts: 8,
      reconnectionDelay: 2000,
      reconnectionDelayMax: 30_000,
      timeout: 10000,
    })
    // Public Notification 1 does not carry the private member proof. Its
    // callbacks are usable only inside this fully bound, uncancelled lifetime.
    const live = () => !stopped && !captured.signal.aborted && captured.current()
    const timers = () => {
      if (expiryTimer !== null) clearTimeout(expiryTimer)
      if (authTimer !== null) clearTimeout(authTimer)
      expiryTimer = null; authTimer = null
    }
    const disconnected = () => setState((previous) => previous.scope === captured ? { ...previous, connected: false } : previous)
    const stop = () => {
      if (stopped) return
      stopped = true
      timers()
      authenticated = null
      captured.signal.removeEventListener("abort", stop)
      socket.io.reconnection(false)
      socket.removeAllListeners()
      socket.disconnect()
      for (const id of toastIds) toast.dismiss(id)
      toastIds.clear(); seenIds.clear()
      // Rejected/expired socket authentication keeps the current HTTP inbox
      // fallback. Only a retired identity owns cancellation of its old queries.
      if (!captured.current()) {
        const replacement = getNotificationContext()
        if (!replacement || captured.key.some((part, index) => replacement.key[index] !== part)) {
          void queryClient.cancelQueries({ queryKey: captured.key })
        }
      }
      disconnected()
    }
    captured.signal.addEventListener("abort", stop, { once: true })
    if (captured.signal.aborted) { stop(); return stop }
    const recover = () => {
      if (!live()) return
      // 从 DB 恢复；fallback 列表即使铃铛未展开也补拉，绝不把这批记录转为 Toast。
      void Promise.allSettled([
        queryClient.invalidateQueries({ queryKey: captured.key, refetchType: "all" }),
        queryClient.fetchQuery({ queryKey: [...captured.key, "unread-count"],
          queryFn: ({ signal }) => notificationsApi.unreadCount(notificationRequestOptions(captured, signal)), staleTime: 0 }),
        queryClient.fetchQuery({ queryKey: [...captured.key, "prefs"],
          queryFn: ({ signal }) => notificationsApi.getPrefs(notificationRequestOptions(captured, signal)), staleTime: 0 }),
        queryClient.fetchQuery({ queryKey: [...captured.key, "recovery-list"],
          queryFn: ({ signal }) => notificationsApi.list({ filter: "ALL", page: 1, size: 5 }, notificationRequestOptions(captured, signal)), staleTime: 0 }),
      ])
    }
    socket.on("connect", () => {
      if (!live()) { stop(); return }
      timers()
      authenticated = null
      disconnected()
      authTimer = setTimeout(stop, 10_000)
      socket.emit("authenticate", { token: captured.token })
    })
    socket.on("authenticated", (data: unknown) => {
      if (!live()) return
      if (!isNotificationSocketContext(data, captured) || Object.keys(data).length !== 5) { stop(); return }
      timers()
      authenticated = data
      expiryTimer = setTimeout(stop, Math.max(1, Date.parse(data.expiresAt) - Date.now()))
      setState((previous) => previous.scope === captured ? { ...previous, connected: true } : previous)
      recover()
    })
    socket.on("unauthorized", stop)
    socket.on("disconnect", () => {
      timers()
      authenticated = null
      disconnected()
    })
    socket.on("connect_error", disconnected)

    socket.on("notification", async (data: unknown) => {
      if (!live() || !authenticated || Date.parse(authenticated.expiresAt) <= Date.now() ||
          !isRealtimeNotification(data, captured) || Object.keys(data).length !== 12) return
      const n = data
      if (seenIds.has(n.id)) return
      seenIds.add(n.id)
      const type = n.type
      const meta = TOAST_META[type]
      const title = n.title.trim()
      const link = notificationLink(n.link.trim(), window.location.origin)
      const description = n.content.trim() || undefined

      // count 由 DB 查询返回，不根据 websocket 事件自行加一。
      void queryClient.invalidateQueries({ queryKey: captured.key, refetchType: "all" })
      setState((previous) => previous.scope === captured ?
        { ...previous, lastNotificationAt: Math.max(Date.now(), previous.lastNotificationAt + 1) } : previous)

      let prefs = queryClient.getQueryData<NotificationPrefs>([...captured.key, "prefs"])
      if (!prefs) {
        try { prefs = await queryClient.fetchQuery({ queryKey: [...captured.key, "prefs"],
          queryFn: ({ signal }) => notificationsApi.getPrefs(notificationRequestOptions(captured, signal)), staleTime: 30_000 }) }
        catch { return }
      }
      if (!prefs || !live() || !authenticated || Date.parse(authenticated.expiresAt) <= Date.now() || prefs.mutedTypes.includes(type) ||
          (prefs.dndEnabled && inDndWindow(prefs.dndStart, prefs.dndEnd))) return

      /* ---- ③ toast 节流 ---- */
      const now = Date.now()
      for (const [t, at] of toastTitleAt) {
        if (now - at >= TOAST_DEDUP_MS) toastTitleAt.delete(t)
      }
      toastWindow = toastWindow.filter((at) => now - at < TOAST_DEDUP_MS)
      if (toastTitleAt.has(title) || toastWindow.length >= TOAST_MAX_PER_WINDOW) return
      toastTitleAt.set(title, now)
      toastWindow.push(now)
      const go = link ? () => { if (live()) navigateRef.current(link) } : undefined
      const Icon = meta.icon
      let toastId: string | number
      toastId = toast(title, {
        description,
        icon: <Icon className={cn("size-4", meta.cls)} />,
        duration: 6000,
        action: go ? { label: "查看", onClick: go } : undefined,
        onDismiss: () => toastIds.delete(toastId),
        onAutoClose: () => toastIds.delete(toastId),
      })
      toastIds.add(toastId)
    })

    return stop
  }, [scope, queryClient])

  const active = !!scope && scope.current() && state.scope === scope
  return { connected: active && state.connected, lastNotificationAt: active ? state.lastNotificationAt : 0,
    prefs: scope?.current() ? prefsQuery.data : undefined }
}
