/** 通知展示及请求共享的捕获上下文；身份来自已验证的平台 profile。 */
export interface NotificationContext {
  token: string
  contextEpoch: number
  userId: string
  tenantId: string
  organizationId: string
  signal: AbortSignal
  key: readonly ["notifications", string, string, string, number]
  current: () => boolean
}

interface AuthSnapshot {
  token: string | null
  contextEpoch: number
  contextSwitching: boolean
  profile: { user: { id: string }; tenant: { id: string }; organization: { id: string } } | null
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
  const matches = (candidate: NotificationContext, state: AuthSnapshot) => !state.contextSwitching &&
    state.token === candidate.token && state.contextEpoch === candidate.contextEpoch &&
    state.profile?.user.id === candidate.userId && state.profile?.tenant.id === candidate.tenantId &&
    state.profile?.organization.id === candidate.organizationId
  const refresh = () => {
    const state = getState()
    if (scope && matches(scope, state)) return
    const previous = controller
    scope = null
    controller = null
    previous?.abort()
    if (state.token && !state.contextSwitching && state.profile?.user.id && state.profile?.tenant.id && state.profile?.organization.id) {
      const lifetime = new AbortController()
      controller = lifetime
      const captured: NotificationContext = {
        token: state.token, contextEpoch: state.contextEpoch, userId: state.profile.user.id,
        tenantId: state.profile.tenant.id, organizationId: state.profile.organization.id,
        signal: lifetime.signal,
        key: Object.freeze(["notifications", state.profile.user.id, state.profile.tenant.id, state.profile.organization.id, state.contextEpoch]),
        current: () => !lifetime.signal.aborted && matches(captured, getState()),
      }
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
