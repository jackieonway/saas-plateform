import { ApiError } from "@/lib/api"
import { isUuidV7, newUuid } from "@/lib/uuid"
import { authStore } from "@/stores/auth"

export type IdentityOperationAction = "create" | "update" | "rotate" | "revoke" | "retire"
export interface PendingIdentityOperation {
  schema: 1
  operationId: string
  action: IdentityOperationAction
  clientLabel: string
  accountId: string
  sessionId: string
  contextKey: string
}
interface Owner { accountId: string; sessionId: string; contextKey: string }
const PREFIX = "platform-identity-operation-v1:"
const FIELDS = ["accountId", "action", "clientLabel", "contextKey", "operationId", "schema", "sessionId"]
const ACTIONS: IdentityOperationAction[] = ["create", "update", "rotate", "revoke", "retire"]

/** Persist only recovery metadata. Mutation bodies and credentials never enter storage. */
export function createIdentityOperationRecovery(storage: () => Storage, owner: () => Owner | null) {
  const key = (contextKey: string) => `${PREFIX}${encodeURIComponent(contextKey)}`
  const parse = (raw: string | null, contextKey: string): PendingIdentityOperation | null => {
    if (!raw) return null
    try {
      const value = JSON.parse(raw) as Record<string, unknown>
      if (!value || typeof value !== "object" || Array.isArray(value)
        || Object.keys(value).sort().join() !== FIELDS.join() || value.schema !== 1
        || !isUuidV7(value.operationId) || !isUuidV7(value.accountId)
        || !ACTIONS.includes(value.action as IdentityOperationAction)
        || typeof value.clientLabel !== "string" || value.clientLabel.length > 120
        || typeof value.sessionId !== "string" || !value.sessionId || value.sessionId.length > 512
        || value.contextKey !== contextKey) return null
      return value as unknown as PendingIdentityOperation
    } catch { return null }
  }
  const read = () => {
    const current = owner()
    if (!current) return null
    try {
      const pending = parse(storage().getItem(key(current.contextKey)), current.contextKey)
      return pending?.accountId === current.accountId ? pending : null
    } catch { throw new ApiError("无法读取管理操作恢复记录，请检查浏览器存储后再操作", "OPERATION_STORAGE_UNAVAILABLE", 0) }
  }
  const begin = (action: IdentityOperationAction, label: string) => {
    const current = owner()
    if (!current) throw new ApiError("当前登录上下文未就绪，请恢复会话后再操作", "OPERATION_OWNER_UNAVAILABLE", 401)
    try {
      const raw = storage().getItem(key(current.contextKey))
      if (raw !== null) {
        const pending = parse(raw, current.contextKey)
        if (!pending || pending.accountId !== current.accountId) throw new ApiError("管理操作恢复记录无法核验，请联系管理员核查，勿重复提交", "OPERATION_STORAGE_INVALID", 409)
        throw new ApiError("已有管理操作等待核验，请查询原操作", "OIDC_REMOTE_RESULT_UNKNOWN", 0, undefined, pending.operationId)
      }
      const pending: PendingIdentityOperation = { schema: 1, operationId: newUuid(), action, clientLabel: label.slice(0, 120),
        accountId: current.accountId, sessionId: current.sessionId, contextKey: current.contextKey }
      const encoded = JSON.stringify(pending)
      storage().setItem(key(current.contextKey), encoded)
      if (storage().getItem(key(current.contextKey)) !== encoded) throw new Error("recovery storage did not persist")
      return pending
    } catch (error) {
      if (error instanceof ApiError) throw error
      throw new ApiError("无法保存管理操作恢复记录，本次请求尚未发送，请检查浏览器存储", "OPERATION_STORAGE_UNAVAILABLE", 0)
    }
  }
  const finish = (operationId: string, contextKey = owner()?.contextKey) => {
    if (!contextKey) return
    try {
      const pending = parse(storage().getItem(key(contextKey)), contextKey)
      if (pending?.operationId === operationId) storage().removeItem(key(contextKey))
    } catch { throw new ApiError("无法清理已完成操作的恢复记录，请保留原操作编号并检查浏览器存储", "OPERATION_STORAGE_UNAVAILABLE", 0) }
  }
  return { read, begin, finish }
}

const recovery = createIdentityOperationRecovery(() => sessionStorage, () => {
  const current = authStore.getState()
  return current.identity && current.sessionId && current.contextKey
    ? { accountId: current.identity.userId, sessionId: current.sessionId, contextKey: current.contextKey }
    : null
})
export const pendingIdentityOperation = recovery.read
export const beginIdentityOperation = recovery.begin
export const finishIdentityOperation = recovery.finish
