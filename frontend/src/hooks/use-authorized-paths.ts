import { useApplicationMenusQueries, usePermissionsQuery } from "@/hooks/use-auth-data"
import { collectAuthorizedPaths, evaluateRouteAccess } from "@/lib/route-policy"
import type { AuthQueryOptions } from "@/lib/auth-queries"
import type { RouteAuthorizationInput } from "@/lib/route-policy"

/** Displayed destinations use the same evidence and policy as the direct guard.
 * Call access() for a concrete dynamic path or embed URL; the path set is for
 * exact static menu entries. A known positive does not wait for unrelated apps.
 */
export function useAuthorizedPaths(options: AuthQueryOptions & { applicationIds?: readonly string[] } = {}) {
  const { profile, switching, epoch, queries } = useApplicationMenusQueries(options)
  const permissionsQuery = usePermissionsQuery(options)
  const policyInput: RouteAuthorizationInput = {
    profile,
    switching,
    permissions: { status: permissionsQuery.status, data: permissionsQuery.data },
    applicationMenus: queries.map(query => ({
      applicationId: query.applicationId,
      status: query.status,
      data: query.data,
    })),
  }
  // Derive from the full current evidence rather than dataUpdatedAt alone:
  // clear(), same-timestamp error transitions and application removal revoke
  // paths immediately, including while observers are disabled.
  const allowedPaths = collectAuthorizedPaths(policyInput)
  const access = (pathname: string, search?: string) => evaluateRouteAccess({ ...policyInput, pathname, search })
  const ready = !!profile && !switching && queries.every(query => query.isSuccess || query.isError)
  return { allowedPaths, ready, access, policyInput, profile, switching, epoch, queries, permissionsQuery }
}
