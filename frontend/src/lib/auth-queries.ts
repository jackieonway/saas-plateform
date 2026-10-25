import { queryOptions } from "@tanstack/react-query"
import { authApi } from "@/api/auth"
import { asArray } from "@/lib/api"
import { sortMenuTree } from "@/lib/tree"
import { useAuthStore } from "@/stores/auth"
import { contextIdentityFromResponse, contextIdentityKey, isContextIdentity, sameContextIdentity, type ContextIdentity } from "@/lib/context-identity"
import type { MenuNode, ProfileResp } from "@/api/types"

/** One authorization cache scope; access-token rotation stays within this epoch. */
export interface AuthQuerySession {
  readonly epoch: number
  readonly token: string | null
  readonly switching: boolean
  readonly identity: ContextIdentity | null
  readonly namespace: string | null
  readonly profileReady: boolean
  readonly applicationIds: readonly string[]
}

export interface AuthQueryOptions { enabled?: boolean }

function boundIdentity(identity: ContextIdentity | null, namespace: string | null): ContextIdentity | null {
  return isContextIdentity(identity) && identity.memberType === "EMPLOYEE" &&
    namespace === contextIdentityKey(identity) ? identity : null
}

export function captureAuthSession(): AuthQuerySession {
  const state = useAuthStore.getState()
  const identity = boundIdentity(state.identity, state.contextKey)
  const profileReady = !!identity && sameContextIdentity(contextIdentityFromResponse(state.profile), identity)
  return Object.freeze({
    epoch: state.contextEpoch,
    token: state.token,
    switching: state.contextSwitching,
    identity: identity ? Object.freeze({ ...identity }) : null,
    namespace: identity ? contextIdentityKey(identity) : null,
    profileReady,
    applicationIds: Object.freeze((profileReady ? state.profile?.applications ?? [] : []).map(app => app.id)),
  })
}

export function isCurrentAuthSession(session: AuthQuerySession): boolean {
  const state = useAuthStore.getState()
  // rotateTokens deliberately keeps the epoch. setSession/logout/context switch
  // advance it. The bound proof and a loaded profile must remain identical even
  // when a caller forgets to advance the epoch during a member/version change.
  const identity = boundIdentity(state.identity, state.contextKey)
  const profileReady = sameContextIdentity(contextIdentityFromResponse(state.profile), identity)
  return !!session.token && !session.switching && !!state.token && !state.contextSwitching &&
    state.contextEpoch === session.epoch && !!session.namespace && state.contextKey === session.namespace &&
    sameContextIdentity(session.identity, identity) &&
    (!session.profileReady || profileReady)
}

export const authQueryKeys = {
  profile: (namespace: string | null, epoch: number) => ["profile", namespace, epoch] as const,
  menus: (applicationId: string | null | undefined, namespace: string | null, epoch: number) => ["auth-menus", applicationId ?? null, namespace, epoch] as const,
  permissions: (namespace: string | null, epoch: number) => ["permissions", namespace, epoch] as const,
}

function queryCurrent(session: AuthQuerySession, signal: AbortSignal, applicationId?: string, requireProfile = false): void {
  const state = useAuthStore.getState()
  const current = isCurrentAuthSession(session) && (!requireProfile || (session.profileReady &&
    sameContextIdentity(contextIdentityFromResponse(state.profile), session.identity))) && (applicationId === undefined ||
    (!!applicationId && session.applicationIds.includes(applicationId) &&
      useAuthStore.getState().profile?.applications.some(app => app.id === applicationId)))
  if (signal.aborted || !current) throw new DOMException("登录授权上下文已变化", "AbortError")
}

async function runAuthQuery<T>(session: AuthQuerySession, signal: AbortSignal, request: () => Promise<T>, applicationId?: string, requireProfile = false): Promise<T> {
  queryCurrent(session, signal, applicationId, requireProfile)
  try {
    const result = await request()
    queryCurrent(session, signal, applicationId, requireProfile)
    return result
  } catch (error) {
    queryCurrent(session, signal, applicationId, requireProfile)
    throw error
  }
}

function retryAuthorizationRead(failureCount: number, error: Error): boolean {
  if (failureCount >= 1 || error.name === "AbortError") return false
  const status = "status" in error ? error.status : undefined
  return status === undefined || status === 0 || status === 429 || (typeof status === "number" && status >= 500)
}

const freshness = {
  staleTime: 20_000,
  refetchInterval: 20_000,
  retry: retryAuthorizationRead,
}

export function authProfileQueryOptions(session: AuthQuerySession, options: AuthQueryOptions = {}) {
  return queryOptions({
    ...freshness,
    queryKey: authQueryKeys.profile(session.namespace, session.epoch),
    queryFn: ({ signal }) => runAuthQuery(session, signal, async () => {
      const profile: ProfileResp = await authApi.profile(undefined, signal)
      if (!sameContextIdentity(contextIdentityFromResponse(profile), session.identity)) {
        throw new DOMException("平台资料与当前成员证明不一致", "AbortError")
      }
      return profile
    }),
    enabled: options.enabled !== false && !!session.identity && isCurrentAuthSession(session),
  })
}

export function authMenusQueryOptions(applicationId: string | null | undefined, session: AuthQuerySession, options: AuthQueryOptions = {}) {
  return queryOptions({
    ...freshness,
    queryKey: authQueryKeys.menus(applicationId, session.namespace, session.epoch),
    queryFn: ({ signal }) => runAuthQuery(session, signal, async () =>
      sortMenuTree(asArray<MenuNode>(await authApi.menus(applicationId!, undefined, signal), "menus")), applicationId ?? "", true),
    enabled: options.enabled !== false && session.profileReady && isCurrentAuthSession(session) && !!applicationId && session.applicationIds.includes(applicationId),
  })
}

export function authPermissionsQueryOptions(session: AuthQuerySession, options: AuthQueryOptions = {}) {
  return queryOptions({
    ...freshness,
    queryKey: authQueryKeys.permissions(session.namespace, session.epoch),
    queryFn: ({ signal }) => runAuthQuery(session, signal, () => authApi.permissions(undefined, signal), undefined, true),
    enabled: options.enabled !== false && session.profileReady && isCurrentAuthSession(session),
  })
}
