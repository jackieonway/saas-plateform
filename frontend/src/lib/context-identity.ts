/** Full identity comes only from an authenticated server context/profile.
 * JWT claims and legacy user/organization preferences cannot fill missing proof. */
export interface ContextIdentity {
  readonly userId: string
  readonly tenantId: string
  readonly organizationId: string
  readonly membershipId: string
  readonly memberType: "EMPLOYEE" | "CUSTOMER"
  readonly identityVersion: string
  readonly accountVersion: string
}

const uuidV7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
const positiveVersion = /^[1-9][0-9]*$/
const nonnegativeVersion = /^(0|[1-9][0-9]*)$/
const i64Maximum = "9223372036854775807"

function version(value: unknown, positive: boolean): value is string {
  return typeof value === "string" && (positive ? positiveVersion : nonnegativeVersion).test(value) &&
    (value.length < i64Maximum.length || value.length === i64Maximum.length && value <= i64Maximum)
}

export function isContextIdentity(value: unknown): value is ContextIdentity {
  if (!value || typeof value !== "object") return false
  const identity = value as Record<string, unknown>
  return ["userId", "tenantId", "organizationId", "membershipId"].every(key =>
    typeof identity[key] === "string" && uuidV7.test(identity[key])) &&
    (identity.memberType === "EMPLOYEE" || identity.memberType === "CUSTOMER") &&
    version(identity.identityVersion, true) && version(identity.accountVersion, false)
}

export function contextIdentityFromResponse(value: unknown): ContextIdentity | null {
  if (!value || typeof value !== "object") return null
  const response = value as Record<string, unknown>
  const id = (field: string) => {
    const object = response[field]
    return object && typeof object === "object" ? (object as Record<string, unknown>).id : undefined
  }
  const identity = { userId: id("user"), tenantId: id("tenant"), organizationId: id("organization"),
    membershipId: response.membershipId, memberType: response.memberType,
    identityVersion: response.identityVersion, accountVersion: response.accountVersion }
  return isContextIdentity(identity) ? Object.freeze(identity) : null
}

export function requireContextIdentity(value: unknown): ContextIdentity {
  const identity = contextIdentityFromResponse(value)
  if (!identity || identity.memberType !== "EMPLOYEE") throw new Error("当前控制台需要有效的员工成员证明，请重新登录")
  return identity
}

/** Context/cache/session namespaces always include the concrete member and generation. */
export function contextIdentityKey(identity: ContextIdentity): string {
  return JSON.stringify([identity.userId, identity.tenantId, identity.organizationId,
    identity.membershipId, identity.memberType, identity.identityVersion, identity.accountVersion])
}

export function sameContextIdentity(first: ContextIdentity | null, second: ContextIdentity | null): boolean {
  return !!first && !!second && contextIdentityKey(first) === contextIdentityKey(second)
}

export interface MembershipSelection {
  userId: string
  tenantId: string
  organizationId: string
  membershipId: string
  memberType: "EMPLOYEE" | "CUSTOMER"
  identityVersion: string
  status: "ENABLED" | "DISABLED"
  consoleSelectable: boolean
  selectionDisabledReason: string | null
}

export function membershipSelectionKey(member: Pick<MembershipSelection,
  "userId" | "tenantId" | "organizationId" | "membershipId" | "memberType" | "identityVersion">): string {
  return JSON.stringify([member.userId, member.tenantId, member.organizationId,
    member.membershipId, member.memberType, member.identityVersion])
}

export function canSelectConsoleMembership(member: MembershipSelection): boolean {
  return member.consoleSelectable === true && member.selectionDisabledReason === null && member.status === "ENABLED" && member.memberType === "EMPLOYEE" &&
    [member.userId, member.tenantId, member.organizationId, member.membershipId].every(id => typeof id === "string" && uuidV7.test(id)) &&
    version(member.identityVersion, true)
}

export function membershipSelectionDisabledReason(member: MembershipSelection): string | null {
  if (member.status !== "ENABLED") return "该成员身份已停用"
  if (member.memberType === "CUSTOMER" || member.selectionDisabledReason === "CLIENT_MEMBER_TYPE_NOT_ALLOWED") return "客户身份请使用客户入口"
  return canSelectConsoleMembership(member) ? null : "当前入口不可选择该身份"
}
