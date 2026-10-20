import type { NotificationRecord, NotificationSocketContext, RealtimeNotification } from "../api/types"
import { NOTIFICATION_CONTRACT_VERSION } from "../../../examples/micro-app-sdk/generated/protocol.mjs"

export interface NotificationIdentity { userId: string; tenantId: string; organizationId: string }
const types = new Set(["INFO", "SUCCESS", "WARNING", "SECURITY"])
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object"
const text = (value: unknown): value is string => typeof value === "string"
const id = (value: unknown): value is string => text(value) && value.length > 0 && value.length <= 128
const date = (value: unknown): value is string => text(value) && Number.isFinite(Date.parse(value))
const boundedText = (value: unknown, max: number) => text(value) && value.length <= max * 2 && Array.from(value).length <= max

export function notificationBelongsTo(value: NotificationIdentity, identity: NotificationIdentity): boolean {
  return value.userId === identity.userId && value.tenantId === identity.tenantId && value.organizationId === identity.organizationId
}

export function isNotificationSocketContext(value: unknown, identity: NotificationIdentity, now = Date.now()): value is NotificationSocketContext {
  return object(value) && value.contractVersion === NOTIFICATION_CONTRACT_VERSION && id(value.userId) && id(value.tenantId) && id(value.organizationId) &&
    date(value.expiresAt) && Date.parse(value.expiresAt) > now && notificationBelongsTo(value as unknown as NotificationIdentity, identity)
}

/** HTTP 与 socket 都必须具备完整接收归属；legacy 行不属于当前收件箱。 */
export function isNotificationRecord(value: unknown): value is NotificationRecord {
  if (!object(value) || !id(value.id) || !id(value.userId) || !id(value.tenantId) || !id(value.organizationId) ||
      !types.has(value.type as string) || !boundedText(value.title, 200) || !(value.title as string).trim() ||
      !boundedText(value.content, 8192) || !boundedText(value.link, 300) || typeof value.isRead !== "boolean" ||
      !(value.readAt === null || date(value.readAt)) || !date(value.createdAt) ||
      !(value.applicationName === null || text(value.applicationName))) return false
  if (value.sourceKind === "APP") return id(value.applicationId) && id(value.publicationId)
  return value.sourceKind === "SYSTEM" && value.applicationId === null && value.publicationId === null
}

export function isRealtimeNotification(value: unknown, identity: NotificationIdentity): value is RealtimeNotification {
  return object(value) && value.contractVersion === NOTIFICATION_CONTRACT_VERSION && id(value.id) && id(value.userId) && id(value.tenantId) && id(value.organizationId) &&
    types.has(value.type as string) && boundedText(value.title, 200) && !!(value.title as string).trim() &&
    boundedText(value.content, 8192) && boundedText(value.link, 300) && date(value.createdAt) &&
    ((value.applicationId === null && value.publicationId === null) || (id(value.applicationId) && id(value.publicationId))) &&
    notificationBelongsTo(value as unknown as NotificationIdentity, identity)
}

/** 通知入口只导航宿主内部路径，拒绝协议、反斜线、编码控制字符与目录穿越。 */
export function notificationLink(value: string, origin: string): string | null {
  if (!value || value.length > 600 || !value.startsWith("/") || value.startsWith("//") || /[\\\u0000-\u001f\u007f]/.test(value)) return null
  try {
    const url = new URL(value, origin)
    const decoded = decodeURIComponent(url.pathname)
    if (url.origin !== origin || /[\\\u0000-\u001f\u007f]/.test(decoded) || decoded.startsWith("//") ||
        decoded.split("/").some((part) => part === "." || part === "..")) return null
    return url.pathname + url.search + url.hash
  } catch { return null }
}
