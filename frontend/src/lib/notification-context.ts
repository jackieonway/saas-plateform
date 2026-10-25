import { contextIdentityFromResponse, contextIdentityKey, isContextIdentity, sameContextIdentity, type ContextIdentity } from "./context-identity.ts"

/** 通知展示及请求共享的捕获上下文；profile 必须与会话成员证明精确一致。 */
export interface NotificationContext extends ContextIdentity {
  token: string
  contextEpoch: number
  namespace: string
  signal: AbortSignal
  key: readonly ["notifications", string, string, string, string, "EMPLOYEE" | "CUSTOMER", string, string, number]
  current: () => boolean
}

interface AuthSnapshot {
  token: string | null
  contextEpoch: number
  contextSwitching: boolean
  identity: ContextIdentity | null
  contextKey: string | null
  profile: unknown
}

export function notificationAbortError(): DOMException {
  return new DOMException("Notification context changed", "AbortError")
}

interface NotificationRequestGuard { bearer: string; signal: AbortSignal; current: () => boolean }

/** 传输忽略取消或响应正文迟到时，仍不能把结果交给另一上下文。 */
export async function runNotificationRequest<T>(options: NotificationRequestGuard, request: () => Promise<T>): Promise<T> {
  const check = () => {
    if (!options || !options.bearer || options.signal.aborted || !options.current()) throw notificationAbortError()
  }
  check()
  try { const result = await request(); check(); return result }
  catch (error) { check(); throw error }
}

/** 同步订阅会在组织切换 action 内 abort，而不等待 React 下一次 effect 清理。 */
export function createNotificationContextStore(
  getState: () => AuthSnapshot,
  subscribeAuth: (listener: () => void) => () => void,
) {
  let scope: NotificationContext | null = null
  let controller: AbortController | null = null
  const listeners = new Set<() => void>()
  const identity = (state: AuthSnapshot): ContextIdentity | null => isContextIdentity(state.identity) &&
    state.contextKey === contextIdentityKey(state.identity) &&
    sameContextIdentity(contextIdentityFromResponse(state.profile), state.identity) ? state.identity : null
  const matches = (candidate: NotificationContext, state: AuthSnapshot) => !state.contextSwitching &&
    state.token === candidate.token && state.contextEpoch === candidate.contextEpoch &&
    state.contextKey === candidate.namespace && sameContextIdentity(candidate, identity(state))
  const refresh = () => {
    const state = getState()
    if (scope && matches(scope, state)) return
    const previous = controller
    scope = null
    controller = null
    previous?.abort()
    const proof = identity(state)
    if (state.token && !state.contextSwitching && proof) {
      const lifetime = new AbortController()
      controller = lifetime
      const captured: NotificationContext = Object.freeze({
        ...proof, token: state.token, contextEpoch: state.contextEpoch, namespace: contextIdentityKey(proof),
        signal: lifetime.signal,
        key: Object.freeze(["notifications", proof.userId, proof.tenantId, proof.organizationId,
          proof.membershipId, proof.memberType, proof.identityVersion, proof.accountVersion, state.contextEpoch] as const),
        current: () => !lifetime.signal.aborted && matches(captured, getState()),
      })
      scope = captured
    }
    for (const listener of [...listeners]) listener()
  }
  refresh()
  const unsubscribe = subscribeAuth(refresh)
  return {
    getSnapshot: () => scope,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => listeners.delete(listener) },
    dispose: () => { unsubscribe(); controller?.abort(); scope = null; controller = null; listeners.clear() },
  }
}
