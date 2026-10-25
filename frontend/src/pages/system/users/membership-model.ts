import { isUuidV7 } from "@/lib/uuid"
import type {
  CreateMembershipBody, DeleteMembershipBody, IdentityTransitionBody,
  MembershipCapabilities, MembershipDeleteResult, MembershipMemberType,
  MembershipPage, MembershipRecord, MembershipStatus, MembershipStatusBody,
  MembershipTransitionResult,
} from "@/api/memberships"

export class MembershipValidationError extends Error {
  constructor(message: string) { super(message); this.name = "MembershipValidationError" }
}

function invalid(message = "成员关系数据无效，请重新读取当前状态"): never {
  throw new MembershipValidationError(message)
}

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) invalid()
  return value as Record<string, unknown>
}

function exact(value: unknown, keys: readonly string[]): Record<string, unknown> {
  const result = object(value), actual = Object.keys(result)
  if (actual.length !== keys.length || keys.some(key => !Object.hasOwn(result, key))) invalid()
  return result
}

export function membershipId(value: unknown): string {
  if (!isUuidV7(value)) invalid("账号、组织与成员标识须为有效的 UUID v7")
  return value.toLowerCase()
}

function memberType(value: unknown): MembershipMemberType {
  if (value !== "EMPLOYEE" && value !== "CUSTOMER") invalid("成员类型无效")
  return value
}

function status(value: unknown): MembershipStatus {
  if (value !== "ENABLED" && value !== "DISABLED") invalid("成员状态无效")
  return value
}

function integer(value: unknown, minimum: number): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < minimum) invalid("成员版本或分页数字无效，请重新读取")
  return value
}

function text(value: unknown): string {
  if (typeof value !== "string") invalid()
  return value
}

function hasControl(value: string): boolean {
  return [...value].some(character => {
    const code = character.codePointAt(0)!
    return code < 32 || (code >= 127 && code <= 159)
  })
}

function reason(value: unknown): string {
  const result = text(value).trim()
  if (!result || [...result].length > 500 || hasControl(result)) invalid("请填写 1～500 字符且不含控制字符的实际变更原因")
  return result
}

function customerBinding(input: Record<string, unknown>, targetType: MembershipMemberType) {
  if (targetType !== "CUSTOMER") return {}
  const source = text(input.bindingSource ?? "").trim()
  if (input.accountBindingConfirmed !== true || !source || [...source].length > 200 || hasControl(source)
    || [...source].some(character => character.codePointAt(0) === 0x2028 || character.codePointAt(0) === 0x2029)) {
    invalid("客户身份须明确确认账号归属，并填写最多 200 字符的单行绑定依据")
  }
  return { accountBindingConfirmed: true, bindingSource: source }
}

const membershipKeys = ["id", "userId", "organizationId", "memberType", "status", "identityVersion", "departmentId", "position", "updatedAt"] as const

function record(value: Record<string, unknown>): MembershipRecord {
  const type = memberType(value.memberType)
  const departmentId = value.departmentId === null ? null : membershipId(value.departmentId)
  const position = text(value.position), updatedAt = text(value.updatedAt)
  if (!updatedAt || !Number.isFinite(Date.parse(updatedAt))) invalid()
  if (type === "CUSTOMER" && (departmentId !== null || position !== "")) invalid("客户身份不得携带员工部门或岗位")
  return {
    id: membershipId(value.id), userId: membershipId(value.userId), organizationId: membershipId(value.organizationId),
    memberType: type, status: status(value.status), identityVersion: integer(value.identityVersion, 1),
    departmentId, position, updatedAt,
  }
}

export function parseMembershipRecord(value: unknown): MembershipRecord {
  return record(exact(value, membershipKeys))
}

export function parseMembershipCapabilities(value: unknown): MembershipCapabilities {
  const input = exact(value, ["lifecycleEnabled", "customerMembershipsEnabled", "dualMembershipsEnabled"])
  for (const key of ["lifecycleEnabled", "customerMembershipsEnabled", "dualMembershipsEnabled"]) {
    if (typeof input[key] !== "boolean") invalid("成员生命周期能力响应无效")
  }
  return {
    lifecycleEnabled: input.lifecycleEnabled as boolean,
    customerMembershipsEnabled: input.customerMembershipsEnabled as boolean,
    dualMembershipsEnabled: input.dualMembershipsEnabled as boolean,
  }
}

export function parseMembershipPage(value: unknown): MembershipPage {
  const input = exact(value, ["records", "total", "page", "size"])
  if (!Array.isArray(input.records)) invalid()
  const rows = input.records.map(parseMembershipRecord), total = integer(input.total, 0)
  const page = integer(input.page, 1), size = integer(input.size, 1)
  if (size > 200 || rows.length > size || rows.length > total || new Set(rows.map(row => row.id)).size !== rows.length) invalid()
  return { records: rows, total, page, size }
}

export function parseMembershipTransitionResult(value: unknown): MembershipTransitionResult {
  const input = exact(value, [...membershipKeys, "sourceMembershipId"])
  return { ...record(input), sourceMembershipId: membershipId(input.sourceMembershipId) }
}

export function parseMembershipDeleteResult(value: unknown): MembershipDeleteResult {
  const input = exact(value, ["id", "deleted", "identityVersion"])
  if (input.deleted !== true) invalid()
  return { id: membershipId(input.id), deleted: true, identityVersion: integer(input.identityVersion, 1) }
}

export function canonicalCreateMembershipBody(value: unknown): CreateMembershipBody {
  const input = object(value), type = memberType(input.memberType)
  const position = text(input.position ?? "").trim()
  const departmentId = input.departmentId === undefined || input.departmentId === null ? null : membershipId(input.departmentId)
  if ([...position].length > 50) invalid("岗位名称最多 50 字符")
  if (type === "CUSTOMER" && (departmentId !== null || position !== "")) invalid("客户身份不得绑定员工部门或岗位")
  return {
    userId: membershipId(input.userId), organizationId: membershipId(input.organizationId),
    memberType: type, reason: reason(input.reason),
    ...(type === "EMPLOYEE" ? { departmentId, position } : customerBinding(input, type)),
  }
}

export function canonicalMembershipDeleteBody(value: unknown): DeleteMembershipBody {
  const input = object(value)
  return { expectedIdentityVersion: integer(input.expectedIdentityVersion, 1), reason: reason(input.reason) }
}

export function canonicalMembershipStatusBody(value: unknown): MembershipStatusBody {
  const input = object(value)
  return { ...canonicalMembershipDeleteBody(input), status: status(input.status) }
}

export function canonicalIdentityTransitionBody(value: unknown): IdentityTransitionBody {
  const input = object(value), type = memberType(input.memberType)
  if (input.enableTarget !== undefined && typeof input.enableTarget !== "boolean") invalid("须明确选择是否启用目标身份")
  if (input.enableTarget === true && input.expectedTargetIdentityVersion === undefined) invalid("启用已有目标身份须携带其当前版本")
  return {
    ...canonicalMembershipDeleteBody(input), memberType: type,
    ...(input.expectedTargetIdentityVersion !== undefined ? { expectedTargetIdentityVersion: integer(input.expectedTargetIdentityVersion, 1) } : {}),
    ...(input.enableTarget === true ? { enableTarget: true } : {}),
    ...customerBinding(input, type),
  }
}

export type MembershipWriteAction = "create" | "status" | "delete" | "secondIdentity" | "replaceIdentity"

export function membershipWriteBlockedReason(
  capabilities: MembershipCapabilities | null | undefined,
  action: MembershipWriteAction,
  targetType?: MembershipMemberType,
): string | null {
  if (!capabilities) return "尚未确认服务端成员生命周期能力，请重新读取"
  let gates: MembershipCapabilities
  try { gates = parseMembershipCapabilities(capabilities) } catch { return "成员生命周期能力无效，请重新读取" }
  if (!gates.lifecycleEnabled) return "服务端尚未开放成员生命周期写入"
  if ((action === "create" || action === "secondIdentity" || action === "replaceIdentity")
    && targetType === "CUSTOMER" && !gates.customerMembershipsEnabled) return "服务端尚未开放客户身份创建"
  if ((action === "secondIdentity" || action === "replaceIdentity") && !gates.dualMembershipsEnabled) return "服务端尚未开放同组织身份组合操作"
  return null
}

function requireWritable(capabilities: MembershipCapabilities | null | undefined, action: MembershipWriteAction, targetType?: MembershipMemberType) {
  const blocked = membershipWriteBlockedReason(capabilities, action, targetType)
  if (blocked) invalid(blocked)
}

export function oppositeMemberType(value: MembershipMemberType): MembershipMemberType {
  return memberType(value) === "EMPLOYEE" ? "CUSTOMER" : "EMPLOYEE"
}

export function currentEmployeeMutationBlockedReason(row: MembershipRecord, currentMembershipId?: string): string | null {
  const member = parseMembershipRecord(row)
  return member.memberType === "EMPLOYEE" && currentMembershipId !== undefined && member.id === membershipId(currentMembershipId)
    ? "当前登录使用的员工身份不能自行启停、删除或替换" : null
}

function requireNotCurrentEmployee(row: MembershipRecord, currentMembershipId?: string) {
  const blocked = currentEmployeeMutationBlockedReason(row, currentMembershipId)
  if (blocked) invalid(blocked)
}

export interface CreateMembershipDraft extends CreateMembershipBody {
  /** Optional known rows let the form detect duplicate/second-identity creation before submission. */
  existingMemberships?: readonly MembershipRecord[]
}

export function buildCreateMembership(draft: CreateMembershipDraft, capabilities: MembershipCapabilities | null | undefined): CreateMembershipBody {
  const request = canonicalCreateMembershipBody(draft)
  requireWritable(capabilities, "create", request.memberType)
  if (draft.existingMemberships !== undefined) {
    const peers = draft.existingMemberships.map(parseMembershipRecord)
      .filter(row => row.userId === request.userId && row.organizationId === request.organizationId)
    if (peers.some(row => row.memberType === request.memberType)) invalid("该账号在组织内已有同类型身份，请重新读取")
    if (peers.length && !capabilities?.dualMembershipsEnabled) invalid("服务端尚未开放同组织第二身份创建")
  }
  return request
}

export function buildMembershipStatusChange(
  row: MembershipRecord, nextStatus: MembershipStatus, changeReason: string,
  capabilities: MembershipCapabilities | null | undefined, currentMembershipId?: string,
): MembershipStatusBody {
  const member = parseMembershipRecord(row)
  requireWritable(capabilities, "status")
  requireNotCurrentEmployee(member, currentMembershipId)
  return canonicalMembershipStatusBody({ status: nextStatus, expectedIdentityVersion: member.identityVersion, reason: changeReason })
}

export function buildMembershipDelete(
  row: MembershipRecord, changeReason: string,
  capabilities: MembershipCapabilities | null | undefined, currentMembershipId?: string,
): DeleteMembershipBody {
  const member = parseMembershipRecord(row)
  requireWritable(capabilities, "delete")
  requireNotCurrentEmployee(member, currentMembershipId)
  return canonicalMembershipDeleteBody({ expectedIdentityVersion: member.identityVersion, reason: changeReason })
}

export interface IdentityTransitionIntent {
  mode: "secondIdentity" | "replaceIdentity"
  memberType: MembershipMemberType
  reason: string
  /** null explicitly means discovery found no opposite identity; undefined is never authoritative. */
  target: MembershipRecord | null
  reuseTargetConfirmed?: boolean
  enableTarget?: boolean
  accountBindingConfirmed?: boolean
  bindingSource?: string
}

export function buildIdentityTransition(
  source: MembershipRecord, intent: IdentityTransitionIntent,
  capabilities: MembershipCapabilities | null | undefined, currentMembershipId?: string,
): IdentityTransitionBody {
  const member = parseMembershipRecord(source), targetType = memberType(intent.memberType)
  if (intent.mode !== "secondIdentity" && intent.mode !== "replaceIdentity") invalid("身份组合操作无效")
  requireWritable(capabilities, intent.mode, targetType)
  if (intent.mode === "replaceIdentity") requireNotCurrentEmployee(member, currentMembershipId)
  if (targetType === member.memberType) invalid("目标类型须与源身份不同，不能原地修改成员类型")
  if (!Object.hasOwn(intent, "target") || intent.target === undefined) invalid("请先重新读取同账号同组织的目标身份")
  let targetVersion: number | undefined
  let enableTarget: boolean | undefined
  if (intent.target !== null) {
    const target = parseMembershipRecord(intent.target)
    if (target.id === member.id || target.userId !== member.userId || target.organizationId !== member.organizationId || target.memberType !== targetType) {
      invalid("目标必须是同账号、同组织的另一类型身份")
    }
    if (intent.reuseTargetConfirmed !== true) invalid("请明确确认复用已存在的目标身份")
    if (target.status === "DISABLED" && intent.enableTarget !== true) invalid("目标身份已停用，须明确确认启用")
    targetVersion = target.identityVersion
    enableTarget = target.status === "DISABLED" ? true : undefined
  } else if (intent.enableTarget === true || intent.reuseTargetConfirmed === true) {
    invalid("目标身份不存在，请重新读取后确认创建")
  }
  return canonicalIdentityTransitionBody({
    memberType: targetType, expectedIdentityVersion: member.identityVersion,
    ...(targetVersion !== undefined ? { expectedTargetIdentityVersion: targetVersion } : {}),
    ...(enableTarget ? { enableTarget: true } : {}), reason: intent.reason,
    accountBindingConfirmed: intent.accountBindingConfirmed, bindingSource: intent.bindingSource,
  })
}

/** Legacy bulk writes project employees only. DISABLED rows remain present without a status write. */
export function legacyEmployeeMembershipItems(rows: readonly MembershipRecord[]): {
  organizationId: string; departmentId: string | null; position: string
}[] {
  const members = rows.map(parseMembershipRecord)
  if (new Set(members.map(row => row.userId)).size > 1) invalid("旧员工批量资料须属于同一账号，请重新读取")
  const employees = members.filter(row => row.memberType === "EMPLOYEE")
  if (new Set(employees.map(row => row.organizationId)).size !== employees.length) invalid("旧员工批量资料包含重复组织，请重新读取")
  return employees.map(row => ({ organizationId: row.organizationId, departmentId: row.departmentId, position: row.position }))
}
