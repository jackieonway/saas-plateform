import { api, ApiError } from "@/lib/api"
import {
  assertPreviewMatches, canonicalGrantChange, parseDataGrantOptions,
  parseDataGrantPreview, parseDataGrantSnapshot, parseDataGrantTarget, sameGrantTarget,
} from "@/lib/data-grants"

export type DataGrantScope = "SELF" | "ORG" | "ORG_SUBTREE" | "TENANT"

export interface DataGrantTarget {
  tenantId: string
  organizationId: string
  userId: string
  membershipId: string
  applicationId: string
}

export interface DataGrantKey { resource: string; action: string }
export interface DataGrantUpsert extends DataGrantKey { scope: DataGrantScope }
export interface DataGrantCapability extends DataGrantKey { scopes: DataGrantScope[] }
export interface DataGrantRow extends DataGrantUpsert {
  /** Only proposed insertions in a preview have null IDs. */
  id: string | null
  effectiveScope: DataGrantScope
  organizationIds: string[]
}
export interface DataGrantStoredRow extends DataGrantRow { id: string }
export interface DataGrantSnapshot {
  target: DataGrantTarget
  version: string
  grants: DataGrantStoredRow[]
  capabilities: DataGrantCapability[]
}
export interface DataGrantChange {
  target: DataGrantTarget
  expectedVersion: string
  upsert: DataGrantUpsert[]
  remove: DataGrantKey[]
}
export interface DataGrantPreview {
  target: DataGrantTarget
  version: string
  before: DataGrantStoredRow[]
  after: DataGrantRow[]
  changes: { added: DataGrantRow[]; updated: DataGrantStoredRow[]; removed: DataGrantStoredRow[] }
}
export interface DataGrantOptionQuery {
  applicationId?: string
  userId?: string
  keyword?: string
  page?: number
  size?: number
}
export interface DataGrantApplication { id: string; code: string; name: string }
export interface DataGrantOptionRecord {
  target: DataGrantTarget
  username: string
  displayName: string
  tenantName: string
  organizationName: string
  canWrite: boolean
}
export interface DataGrantOptions {
  applications: DataGrantApplication[]
  records: DataGrantOptionRecord[]
  total: number
  page: number
  size: number
}

function responseData<T>(parse: (value: unknown) => T, value: unknown, requestId?: string): T {
  try { return parse(value) } catch {
    throw new ApiError("数据授权响应无效，请重新读取当前状态", "PROTOCOL_ERROR", 502, requestId)
  }
}

/** All operations use the shared platform transport and its session fencing. */
export const dataGrantApi = {
  async options(query: DataGrantOptionQuery = {}, signal?: AbortSignal): Promise<DataGrantOptions> {
    const expected = { ...query }
    const params = new URLSearchParams()
    for (const key of ["applicationId", "userId", "keyword", "page", "size"] as const) {
      const value = expected[key]
      if (value !== undefined && value !== "") params.set(key, String(value))
    }
    let requestId: string | undefined
    const value = await api.get<unknown>(`/api/data-grants/options${params.size ? `?${params}` : ""}`, {
      signal, onResponse: meta => { requestId = meta.requestId },
    })
    return responseData(raw => {
      const result = parseDataGrantOptions(raw)
      if (result.page !== (expected.page ?? 1) || result.size !== (expected.size ?? 20)) throw new Error("Unexpected pagination")
      if (!expected.applicationId && (result.records.length || result.total !== 0)) throw new Error("Unexpected target records")
      for (const record of result.records) {
        if (expected.applicationId && record.target.applicationId !== expected.applicationId.toLowerCase()) throw new Error("Unexpected application")
        if (expected.userId && record.target.userId !== expected.userId.toLowerCase()) throw new Error("Unexpected user")
      }
      return result
    }, value, requestId)
  },

  async read(target: DataGrantTarget, signal?: AbortSignal): Promise<DataGrantSnapshot> {
    const expected = parseDataGrantTarget(target)
    const params = new URLSearchParams(Object.entries(expected))
    let requestId: string | undefined
    const value = await api.get<unknown>(`/api/data-grants?${params}`, {
      signal, onResponse: meta => { requestId = meta.requestId },
    })
    return responseData(raw => {
      const snapshot = parseDataGrantSnapshot(raw)
      if (!sameGrantTarget(snapshot.target, expected)) throw new Error("Unexpected target")
      return snapshot
    }, value, requestId)
  },

  async preview(change: DataGrantChange, signal?: AbortSignal): Promise<DataGrantPreview> {
    const request = canonicalGrantChange(change, true)
    let requestId: string | undefined
    const value = await api.post<unknown>("/api/data-grants/preview", request, {
      signal, onResponse: meta => { requestId = meta.requestId },
    })
    return responseData(raw => {
      const preview = parseDataGrantPreview(raw)
      assertPreviewMatches(request, preview)
      return preview
    }, value, requestId)
  },

  async change(change: DataGrantChange, signal?: AbortSignal): Promise<{ snapshot: DataGrantSnapshot; requestId?: string }> {
    const request = canonicalGrantChange(change, true)
    let requestId: string | undefined
    const value = await api.patch<unknown>("/api/data-grants", request, {
      signal, onResponse: meta => { requestId = meta.requestId },
    })
    const snapshot = responseData(raw => {
      const result = parseDataGrantSnapshot(raw)
      if (!sameGrantTarget(result.target, request.target)) throw new Error("Unexpected target")
      return result
    }, value, requestId)
    return { snapshot, requestId }
  },
}
