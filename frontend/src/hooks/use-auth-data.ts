import { useQueries, useQuery } from "@tanstack/react-query"
import { useAuthStore } from "@/stores/auth"
import { switchPlatformContext } from "@/lib/api"
import { authMenusQueryOptions, authPermissionsQueryOptions, authProfileQueryOptions, captureAuthSession } from "@/lib/auth-queries"
import type { AuthQueryOptions } from "@/lib/auth-queries"

function useAuthQuerySession() {
  useAuthStore((s) => s.token)
  useAuthStore((s) => s.contextEpoch)
  useAuthStore((s) => s.contextSwitching)
  useAuthStore((s) => s.profile)
  useAuthStore((s) => s.identity)
  useAuthStore((s) => s.contextKey)
  return captureAuthSession()
}

export function useProfileQuery(options: AuthQueryOptions = {}) {
  return useQuery(authProfileQueryOptions(useAuthQuerySession(), options))
}

export function useMenusQuery(applicationId: string | null | undefined, options: AuthQueryOptions = {}) {
  return useQuery(authMenusQueryOptions(applicationId, useAuthQuerySession(), options))
}

export function usePermissionsQuery(options: AuthQueryOptions = {}) {
  return useQuery(authPermissionsQueryOptions(useAuthQuerySession(), options))
}

/** Observers and imperative navigation consume exactly the same menu options. */
export function useApplicationMenusQueries(options: AuthQueryOptions & { applicationIds?: readonly string[] } = {}) {
  const session = useAuthQuerySession()
  const storedProfile = useAuthStore((s) => s.profile)
  const profile = session.profileReady ? storedProfile : null
  const apps = (profile?.applications ?? []).filter(app => !options.applicationIds || options.applicationIds.includes(app.id))
  const queries = useQueries({
    queries: apps.map(app => authMenusQueryOptions(app.id, session, options)),
  })
  return {
    profile,
    epoch: session.epoch,
    switching: session.switching,
    queries: queries.map((query, index) => ({ ...query, applicationId: apps[index].id })),
  }
}

/** Refresh, old-view retirement and context replacement share one cross-page transaction. */
export function useSwitchContext() {
  return (organizationId: string, membershipId: string) => switchPlatformContext(organizationId, membershipId)
}
