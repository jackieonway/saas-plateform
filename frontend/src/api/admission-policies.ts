import { api } from "@/lib/api"
import { isUuidV7 } from "@/lib/uuid"

export type AdmissionMemberType = "EMPLOYEE" | "CUSTOMER"
export type AdmissionSubjectMode = "USER" | "MACHINE" | "LEGACY_MIXED"
export interface AdmissionPolicyTarget {
  kind: "application" | "client"
  id: string
  name: string
  fixedEmployee?: boolean
}
export interface AdmissionPolicySummary {
  allowedMemberTypes?: AdmissionMemberType[]
  accessPolicyVersion?: number
  subjectMode?: AdmissionSubjectMode
}
export interface AdmissionPolicy {
  id: string
  status: "ENABLED" | "DISABLED"
  allowedMemberTypes: AdmissionMemberType[]
  accessPolicyVersion: number
  subjectMode?: AdmissionSubjectMode
}
export interface AdmissionPolicyChange {
  allowedMemberTypes: readonly AdmissionMemberType[]
  accessPolicyVersion: number
  reason: string
}

export const admissionMemberLabels: Record<AdmissionMemberType, string> = { EMPLOYEE: "员工", CUSTOMER: "客户" }
export const admissionSubjectLabels: Record<AdmissionSubjectMode, string> = { USER: "人员登录", MACHINE: "机器凭据", LEGACY_MIXED: "历史混合用途" }
const memberOrder: readonly AdmissionMemberType[] = ["EMPLOYEE", "CUSTOMER"]

function endpoint(target: AdmissionPolicyTarget): string {
  if (!isUuidV7(target.id)) throw new Error("准入策略目标无效，请刷新列表")
  return target.kind === "application" ? `/api/applications/${target.id}/access-policy` : `/api/oauth/clients/${target.id}/access-policy`
}

export function parseAdmissionPolicy(value: unknown, target: AdmissionPolicyTarget): AdmissionPolicy {
  if (!value || typeof value !== "object") throw new Error("准入策略响应无效")
  const record = value as Record<string, unknown>
  if (record.id !== target.id || !isUuidV7(record.id) || !["ENABLED", "DISABLED"].includes(record.status as string) ||
      !Number.isSafeInteger(record.accessPolicyVersion) || (record.accessPolicyVersion as number) < 1 ||
      !Array.isArray(record.allowedMemberTypes) || record.allowedMemberTypes.some(type => !memberOrder.includes(type)) ||
      new Set(record.allowedMemberTypes).size !== record.allowedMemberTypes.length) throw new Error("准入策略响应无效")
  const subjectMode = record.subjectMode as AdmissionSubjectMode | undefined
  if (target.kind === "application" && subjectMode !== undefined) throw new Error("应用策略不能使用客户端主体用途")
  if (target.kind === "client" && !["USER", "MACHINE", "LEGACY_MIXED"].includes(subjectMode ?? "")) throw new Error("客户端用途未经过验证")
  if ((subjectMode === "MACHINE") !== (record.allowedMemberTypes.length === 0)) throw new Error("准入策略与主体用途不一致")
  const allowed = record.allowedMemberTypes as AdmissionMemberType[]
  return { id: target.id, status: record.status as AdmissionPolicy["status"], subjectMode,
    accessPolicyVersion: record.accessPolicyVersion as number,
    allowedMemberTypes: memberOrder.filter(type => allowed.includes(type)) }
}

export function admissionPolicyBody(change: AdmissionPolicyChange): AdmissionPolicyChange {
  const reason = change.reason.trim()
  if (!Number.isSafeInteger(change.accessPolicyVersion) || change.accessPolicyVersion < 1) throw new Error("请先读取当前策略版本")
  if (!isAdmissionReasonValid(reason)) throw new Error("变更原因须为 1～500 字且不含控制字符")
  if (change.allowedMemberTypes.some(type => !memberOrder.includes(type))) throw new Error("成员类型无效")
  return { allowedMemberTypes: memberOrder.filter(type => change.allowedMemberTypes.includes(type)), accessPolicyVersion: change.accessPolicyVersion, reason }
}

export function isAdmissionReasonValid(value: string): boolean {
  const characters = Array.from(value.trim())
  return characters.length >= 1 && characters.length <= 500 && !characters.some(character => {
    const code = character.codePointAt(0)!
    return code < 32 || code >= 127 && code <= 159
  })
}

export const admissionPoliciesApi = {
  read: async (target: AdmissionPolicyTarget, signal?: AbortSignal) => parseAdmissionPolicy(await api.get<unknown>(endpoint(target), { signal }), target),
  save: async (target: AdmissionPolicyTarget, change: AdmissionPolicyChange) =>
    parseAdmissionPolicy(await api.put<unknown>(endpoint(target), admissionPolicyBody(change)), target),
}
