import { queryOptions } from "@tanstack/react-query"
import { authApi } from "@/api/auth"
import { asArray } from "@/lib/api"
import { sortMenuTree } from "@/lib/tree"
import { useAuthStore } from "@/stores/auth"
import type { MenuNode, ProfileResp } from "@/api/types"

/** One authorization cache scope; access-token rotation stays within this epoch. */
export interface AuthQuerySession {
  readonly epoch: number
  readonly token: string | null
  readonly switching: boolean
  readonly identity: string | null
  readonly applicationIds: readonly string[]
}

export interface AuthQueryOptions { enabled?: boolean }

function profileIdentity(profile: ProfileResp | null): string | null {
  return profile ? JSON.stringify([profile.user.id, profile.tenant.id, profile.organization.id]) : null
}

export function captureAuthSession(): AuthQuerySession {
  const state = useAuthStore.getState()
  return Object.freeze({
    epoch: state.contextEpoch,
    token: state.token,
    switching: state.contextSwitching,
    identity: profileIdentity(state.profile),
    applicationIds: Object.freeze((state.profile?.applications ?? []).map(app => app.id)),
  })
}

export function isCurrentAuthSession(session: AuthQuerySession): boolean {
  const state = useAuthStore.getState()
  // rotateTokens deliberately keeps the epoch. setSession/logout/context switch
  // advance it, and identity checking also catches mutation of a captured profile.
  return !!session.token && !session.switching && !!state.token && !state.contextSwitching &&
    state.contextEpoch === session.epoch &&
    (session.identity === null || profileIdentity(state.profile) === session.identity)
}

export const authQueryKeys = {
  profile: (epoch: number) => ["profile", epoch] as const,
  menus: (applicationId: string | null | undefined, epoch: number) => ["auth-menus", applicationId ?? null, epoch] as const,
  permissions: (epoch: number) => ["permissions", epoch] as const,
}

function queryCurrent(session: AuthQuerySession, signal: AbortSignal, applicationId?: string): void {
  const current = isCurrentAuthSession(session) && (applicationId === undefined ||
    (!!applicationId && session.applicationIds.includes(applicationId) &&
      useAuthStore.getState().profile?.applications.some(app => app.id === applicationId)))
  if (signal.aborted || !current) throw new DOMException("登录授权上下文已变化", "AbortError")
}

async function runAuthQuery<T>(session: AuthQuerySession, signal: AbortSignal, request: () => Promise<T>, applicationId?: string): Promise<T> {
  queryCurrent(session, signal, applicationId)
  try {
    const result = await request()
    queryCurrent(session, signal, applicationId)
    return result
  } catch (error) {
    queryCurrent(session, signal, applicationId)
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
    queryKey: authQueryKeys.profile(session.epoch),
    queryFn: ({ signal }) => runAuthQuery(session, signal, () => authApi.profile(undefined, signal)),
    enabled: options.enabled !== false && !!session.token && !session.switching,
  })
}

export function authMenusQueryOptions(applicationId: string | null | undefined, session: AuthQuerySession, options: AuthQueryOptions = {}) {
  return queryOptions({
    ...freshness,
    queryKey: authQueryKeys.menus(applicationId, session.epoch),
    queryFn: ({ signal }) => runAuthQuery(session, signal, async () =>
      sortMenuTree(asArray<MenuNode>(await authApi.menus(applicationId!, undefined, signal), "menus")), applicationId ?? ""),
    enabled: options.enabled !== false && !!session.token && !session.switching && !!applicationId && session.applicationIds.includes(applicationId),
  })
}

export function authPermissionsQueryOptions(session: AuthQuerySession, options: AuthQueryOptions = {}) {
  return queryOptions({
    ...freshness,
    queryKey: authQueryKeys.permissions(session.epoch),
    queryFn: ({ signal }) => runAuthQuery(session, signal, () => authApi.permissions(undefined, signal)),
    enabled: options.enabled !== false && !!session.token && !session.switching,
  })
}
