import { useSyncExternalStore } from "react"
import { capturePlatformSessionFence, reconcileSessionSync, useAuthStore } from "@/stores/auth"
import { createNotificationContextStore, type NotificationContext } from "@/lib/notification-context"

export type { NotificationContext } from "@/lib/notification-context"
const currentSessionState = () => {
  // A socket callback may precede storage/BroadcastChannel delivery after tab
  // suspension; read the shared version before testing the captured lifetime.
  reconcileSessionSync()
  return useAuthStore.getState()
}
const store = createNotificationContextStore(currentSessionState, (listener) => useAuthStore.subscribe(listener))
export const getNotificationContext = store.getSnapshot

export function useNotificationContext(): NotificationContext | null {
  return useSyncExternalStore(store.subscribe, store.getSnapshot, () => null)
}

/** query/mutation 必须显式带捕获的 bearer，并把 query 的取消联动 context lifetime。 */
export function notificationRequestOptions(scope: NotificationContext, signal?: AbortSignal) {
  if (scope.signal.aborted || signal?.aborted || !scope.current()) throw new DOMException("通知成员上下文已变化", "AbortError")
  return { bearer: scope.token, sessionFence: capturePlatformSessionFence(scope.token), signal: signal ? AbortSignal.any([scope.signal, signal]) : scope.signal,
    current: scope.current, context: { userId: scope.userId, tenantId: scope.tenantId, organizationId: scope.organizationId,
      membershipId: scope.membershipId, memberType: scope.memberType, identityVersion: scope.identityVersion, accountVersion: scope.accountVersion } }
}

if (import.meta.hot) import.meta.hot.dispose(() => store.dispose())
