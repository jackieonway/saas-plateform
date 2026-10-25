import { api, ApiError } from "@/lib/api"
import {
  canonicalCreateMembershipBody, canonicalIdentityTransitionBody,
  canonicalMembershipDeleteBody, canonicalMembershipStatusBody,
  membershipId, parseMembershipCapabilities, parseMembershipDeleteResult,
  parseMembershipPage, parseMembershipRecord, parseMembershipTransitionResult,
} from "@/pages/system/users/membership-model"

/** Lifecycle DTOs are deliberately separate from login choices and legacy employee bulk rows. */
export type MembershipMemberType = "EMPLOYEE" | "CUSTOMER"
export type MembershipStatus = "ENABLED" | "DISABLED"

export interface MembershipRecord {
  id: string
  userId: string
  organizationId: string
  memberType: MembershipMemberType
  status: MembershipStatus
  /** This management API returns an i64 JSON number; unsafe numbers are rejected. */
  identityVersion: number
  departmentId: string | null
  position: string
  updatedAt: string
}

export interface MembershipCapabilities {
  lifecycleEnabled: boolean
  customerMembershipsEnabled: boolean
  dualMembershipsEnabled: boolean
}

export interface MembershipListQuery {
  userId?: string
  organizationId?: string
  tenantId?: string
  memberType?: MembershipMemberType
  status?: MembershipStatus
  page?: number
  size?: number
}

export interface MembershipPage {
  records: MembershipRecord[]
  total: number
  page: number
  size: number
}

export interface CreateMembershipBody {
  userId: string
  organizationId: string
  memberType: MembershipMemberType
  reason: string
  departmentId?: string | null
  position?: string
  accountBindingConfirmed?: boolean
  bindingSource?: string
}

export interface DeleteMembershipBody {
  expectedIdentityVersion: number
  reason: string
}

export interface MembershipStatusBody extends DeleteMembershipBody {
  status: MembershipStatus
}

export interface IdentityTransitionBody extends DeleteMembershipBody {
  memberType: MembershipMemberType
  expectedTargetIdentityVersion?: number
  enableTarget?: boolean
  accountBindingConfirmed?: boolean
  bindingSource?: string
}

export interface MembershipTransitionResult extends MembershipRecord {
  sourceMembershipId: string
}

export interface MembershipDeleteResult {
  id: string
  deleted: true
  identityVersion: number
}

function responseData<T>(parse: (value: unknown) => T, value: unknown, requestId?: string): T {
  try { return parse(value) } catch {
    throw new ApiError("成员关系响应无效，请重新读取当前状态", "PROTOCOL_ERROR", 502, requestId)
  }
}

async function readResponse<T>(
  send: (onResponse: (meta: { requestId?: string }) => void) => Promise<unknown>,
  parse: (value: unknown) => T,
): Promise<T> {
  let requestId: string | undefined
  const value = await send(meta => { requestId = meta.requestId })
  return responseData(parse, value, requestId)
}

function expectId<T extends { id: string }>(value: T, expected: string): T {
  if (value.id !== expected) throw new Error("Unexpected membership")
  return value
}

/** Every operation uses the shared platform transport and its session/context fence. */
export const membershipsApi = {
  capabilities: (signal?: AbortSignal): Promise<MembershipCapabilities> => readResponse(
    onResponse => api.get<unknown>("/api/memberships/capabilities", { signal, onResponse }),
    parseMembershipCapabilities,
  ),

  list(query: MembershipListQuery = {}, signal?: AbortSignal): Promise<MembershipPage> {
    const requested = { ...query }
    const params = new URLSearchParams()
    for (const key of ["userId", "organizationId", "tenantId"] as const) {
      if (requested[key] !== undefined) params.set(key, membershipId(requested[key]))
    }
    if (requested.memberType !== undefined) {
      if (requested.memberType !== "EMPLOYEE" && requested.memberType !== "CUSTOMER") throw new Error("成员类型无效")
      params.set("memberType", requested.memberType)
    }
    if (requested.status !== undefined) {
      if (requested.status !== "ENABLED" && requested.status !== "DISABLED") throw new Error("成员状态无效")
      params.set("status", requested.status)
    }
    for (const key of ["page", "size"] as const) {
      const value = requested[key]
      if (value !== undefined) {
        if (!Number.isSafeInteger(value) || value < 1 || (key === "size" && value > 200)
          || (key === "page" && value > 1_000_000)) throw new Error("分页参数无效")
        params.set(key, String(value))
      }
    }
    const expected = Object.fromEntries(params)
    return readResponse(
      onResponse => api.get<unknown>(`/api/memberships${params.size ? `?${params}` : ""}`, { signal, onResponse }),
      raw => {
        const result = parseMembershipPage(raw)
        if ((requested.page !== undefined && result.page !== requested.page)
          || (requested.size !== undefined && result.size !== requested.size)) throw new Error("Unexpected pagination")
        for (const row of result.records) {
          for (const key of ["userId", "organizationId", "memberType", "status"] as const) {
            if (expected[key] !== undefined && row[key] !== expected[key]) throw new Error("Unexpected membership filter")
          }
        }
        return result
      },
    )
  },

  detail(id: string, signal?: AbortSignal): Promise<MembershipRecord> {
    const expected = membershipId(id)
    return readResponse(
      onResponse => api.get<unknown>(`/api/memberships/${expected}`, { signal, onResponse }),
      raw => expectId(parseMembershipRecord(raw), expected),
    )
  },

  create(body: CreateMembershipBody, signal?: AbortSignal): Promise<MembershipRecord> {
    const request = canonicalCreateMembershipBody(body)
    return readResponse(
      onResponse => api.post<unknown>("/api/memberships", request, { signal, onResponse }),
      raw => {
        const result = parseMembershipRecord(raw)
        if (result.userId !== request.userId || result.organizationId !== request.organizationId
          || result.memberType !== request.memberType) throw new Error("Unexpected membership target")
        return result
      },
    )
  },

  changeStatus(id: string, body: MembershipStatusBody, signal?: AbortSignal): Promise<MembershipRecord> {
    const expected = membershipId(id), request = canonicalMembershipStatusBody(body)
    return readResponse(
      onResponse => api.put<unknown>(`/api/memberships/${expected}/status`, request, { signal, onResponse }),
      raw => {
        const result = expectId(parseMembershipRecord(raw), expected)
        if (result.status !== request.status || result.identityVersion <= request.expectedIdentityVersion) throw new Error("Unexpected membership status")
        return result
      },
    )
  },

  remove(id: string, body: DeleteMembershipBody, signal?: AbortSignal): Promise<MembershipDeleteResult> {
    const expected = membershipId(id), request = canonicalMembershipDeleteBody(body)
    return readResponse(
      onResponse => api.del<unknown>(`/api/memberships/${expected}`, request, { signal, onResponse }),
      raw => {
        const result = expectId(parseMembershipDeleteResult(raw), expected)
        if (result.identityVersion <= request.expectedIdentityVersion) throw new Error("Unexpected membership version")
        return result
      },
    )
  },

  secondIdentity(id: string, body: IdentityTransitionBody, signal?: AbortSignal): Promise<MembershipTransitionResult> {
    return transitionRequest(id, body, "second-identity", signal)
  },

  replaceIdentity(id: string, body: IdentityTransitionBody, signal?: AbortSignal): Promise<MembershipTransitionResult> {
    return transitionRequest(id, body, "replace-identity", signal)
  },
}

function transitionRequest(id: string, body: IdentityTransitionBody, path: string, signal?: AbortSignal) {
  const source = membershipId(id), request = canonicalIdentityTransitionBody(body)
  return readResponse(
    onResponse => api.post<unknown>(`/api/memberships/${source}/${path}`, request, { signal, onResponse }),
    raw => {
      const result = parseMembershipTransitionResult(raw)
      if (result.sourceMembershipId !== source || result.id === source
        || result.memberType !== request.memberType || result.status !== "ENABLED") throw new Error("Unexpected identity transition")
      return result
    },
  )
}
