import { useSyncExternalStore } from "react"
import { useAuthStore } from "@/stores/auth"
import { createNotificationContextStore, type NotificationContext } from "@/lib/notification-context"

export type { NotificationContext } from "@/lib/notification-context"
const store = createNotificationContextStore(useAuthStore.getState, (listener) => useAuthStore.subscribe(listener))
export const getNotificationContext = store.getSnapshot

export function useNotificationContext(): NotificationContext | null {
  return useSyncExternalStore(store.subscribe, store.getSnapshot, () => null)
}

/** query/mutation 必须显式带捕获的 bearer，并把 query 的取消联动 context lifetime。 */
export function notificationRequestOptions(scope: NotificationContext, signal?: AbortSignal) {
  return { bearer: scope.token, signal: signal ? AbortSignal.any([scope.signal, signal]) : scope.signal,
    current: scope.current, context: { userId: scope.userId, tenantId: scope.tenantId, organizationId: scope.organizationId } }
}

if (import.meta.hot) import.meta.hot.dispose(() => store.dispose())
