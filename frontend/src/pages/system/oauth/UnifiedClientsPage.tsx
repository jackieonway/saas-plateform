import * as React from "react"
import { ClientAuthorizationSettings } from "./ClientAuthorizationSettings"
import { tokenSeconds, validTokenSeconds } from "./client-policy"
import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query"
import { useFieldArray, useForm } from "react-hook-form"
import { useSearchParams } from "react-router-dom"
import { z } from "zod"
import {
  AlertCircle,
  ArrowDownUp,
  BadgeCheck,
  Building2,
  Check,
  ChevronDown,
  CircleDot,
  ClipboardCheck,
  Eye,
  Fingerprint,
  KeyRound,
  Layers3,
  Loader2,
  Plus,
  RefreshCw,
  Search,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  Trash2,
  X,
} from "lucide-react"

import { applicationsApi } from "@/api/applications"
import { identityClientsApi } from "@/api/identity-clients"
import type {
  ApplicationAuthProjection,
  ApplicationRecord,
  UnifiedClientListParams,
  UnifiedClientOperation,
  UnifiedClientPayload,
  UnifiedClientRecord,
  UnifiedClientStatus,
  UnifiedContextMode,
  UnifiedRuntimeKind,
  UnifiedSubjectMode,
} from "@/api/types"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { CopyButton, SecretDialog } from "@/components/CopyButton"
import { DataPagination } from "@/components/DataPagination"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Textarea } from "@/components/ui/textarea"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states"
import { StatusBadge } from "@/components/StatusBadge"
import { zResolver } from "@/lib/form"
import { asArray, ApiError } from "@/lib/api"
import { isUuidV7 } from "@/lib/uuid"
import { finishIdentityOperation, pendingIdentityOperation } from "@/lib/identity-operation-recovery"
import { useAuthStore } from "@/stores/auth"
import { toast } from "sonner"

const ALL = "ALL"
const PAGE_SIZES = [10, 20, 50]
const MEMBER_TYPES = ["EMPLOYEE", "CUSTOMER"] as const
const RESOURCE_KEY = /^[a-z][a-z0-9._-]{0,63}$/
const SCOPE_NAME = /^[a-z][a-z0-9._:-]{0,127}$/

const permissionSchema = z.object({
  resourceKey: z.string().trim().min(1, "请输入资源标识").max(64),
  scopesText: z.string().trim().min(1, "至少选择一个 scope").max(1200),
})

const clientFormSchema = z.object({
  applicationId: z.string().trim().min(1, "请选择应用").max(36),
  environment: z.enum(["test", "production"]),
  displayName: z.string().trim().min(1, "请输入客户端名称").max(120),
  subjectMode: z.enum(["USER", "MACHINE"]),
  runtimeKind: z.enum(["WEB_BFF", "SPA", "NATIVE", "SERVICE"]),
  contextMode: z.enum(["ACCOUNT_ONLY", "PLATFORM_CONTEXT", "NONE"]),
  allowedMemberTypes: z.array(z.enum(MEMBER_TYPES)).max(2),
  refreshToken: z.boolean(),
  autoAuthorize: z.boolean(),
  accessTokenTtlSeconds: z.string().refine(raw => validTokenSeconds(raw), "请输入 60～3600 的整数秒数"),
  refreshTokenTtlSeconds: z.string().refine(raw => validTokenSeconds(raw, true), "请输入 300～604800 的整数秒数"),
  clientTokenTtlSeconds: z.string().refine(raw => validTokenSeconds(raw), "请输入 60～3600 的整数秒数"),
  redirectUrisText: z.string().max(32_768),
  postLogoutRedirectUrisText: z.string().max(32_768),
  logoutMode: z.enum(["SHORT_SESSION_ONLINE_CHECK", "BACK_CHANNEL"]),
  backchannelLogoutUri: z.string().trim().max(2048),
  backchannelTargetId: z.string().trim().max(36),
  status: z.enum(["ENABLED", "DISABLED", "RETIRED"]),
  consentGroupId: z.string().trim().max(36),
  permissions: z.array(permissionSchema).min(1, "至少配置一个资源权限").max(64),
}).superRefine((value, ctx) => {
  const issue = (path: (string | number)[], message: string) => ctx.addIssue({ code: "custom", path, message })
  if (!isUuidV7(value.applicationId)) issue(["applicationId"], "应用标识必须是规范 UUIDv7")
  if (value.consentGroupId && !isUuidV7(value.consentGroupId)) issue(["consentGroupId"], "同意组标识必须是规范 UUIDv7")
  if (value.logoutMode === "BACK_CHANNEL") {
    if (value.subjectMode !== "USER" || value.runtimeKind !== "WEB_BFF") issue(["logoutMode"], "只有机密Web BFF人员客户端支持back-channel")
    if (!isUuidV7(value.backchannelTargetId)) issue(["backchannelTargetId"], "填写owner核验的规范目标UUIDv7；任意引用不构成核验")
    try {
      const url = new URL(value.backchannelLogoutUri)
      if (url.protocol !== "https:" || !url.hostname || url.username || url.password || url.search || url.hash || url.href !== value.backchannelLogoutUri || value.backchannelLogoutUri.includes("*")) issue(["backchannelLogoutUri"], "填写无查询参数的精确HTTPS back-channel地址")
    } catch { issue(["backchannelLogoutUri"], "填写无查询参数的精确HTTPS back-channel地址") }
  } else if (value.backchannelLogoutUri || value.backchannelTargetId) issue(["logoutMode"], "短会话合同不配置back-channel地址或目标")

  const redirects = splitLines(value.redirectUrisText)
  const logoutRedirects = splitLines(value.postLogoutRedirectUrisText)
  if (new Set(redirects).size !== redirects.length) issue(["redirectUrisText"], "回调地址不能重复")
  if (new Set(logoutRedirects).size !== logoutRedirects.length) issue(["postLogoutRedirectUrisText"], "退出后返回地址不能重复")
  if (value.subjectMode === "MACHINE") {
    if (value.runtimeKind !== "SERVICE" || value.contextMode !== "NONE" || value.allowedMemberTypes.length !== 0) {
      issue(["subjectMode"], "机器客户端必须是 SERVICE / NONE，且不配置人员身份")
    }
    if (value.refreshToken || redirects.length || logoutRedirects.length) {
      issue(["subjectMode"], "机器客户端不使用 refresh token、回调或退出后返回地址")
    }
  } else {
    if (value.runtimeKind === "SERVICE") issue(["runtimeKind"], "人员客户端请选择 WEB_BFF、SPA 或 NATIVE")
    if (value.contextMode === "NONE") issue(["contextMode"], "人员客户端必须选择登录上下文")
    if (value.contextMode === "ACCOUNT_ONLY" && value.allowedMemberTypes.length) {
      issue(["allowedMemberTypes"], "纯账号登录不配置员工或客户成员")
    }
    if (value.contextMode === "PLATFORM_CONTEXT") {
      if (value.allowedMemberTypes.length !== 1) issue(["allowedMemberTypes"], "业务上下文必须限定一种员工或客户身份")
      if (value.runtimeKind !== "WEB_BFF") issue(["runtimeKind"], "业务上下文只支持机密 Web BFF 客户端")
    }
    if (!redirects.length) issue(["redirectUrisText"], "授权码客户端至少需要一个回调地址")
  }

  for (const [field, values] of [["redirectUrisText", redirects], ["postLogoutRedirectUrisText", logoutRedirects]] as const) {
    for (const raw of values) {
      let url: URL
      try { url = new URL(raw) } catch { issue([field], `不是有效 URL：${raw}`); continue }
      if (url.protocol !== "https:" || !url.hostname || url.username || url.password || url.hash || url.href !== raw || raw.includes("*")) {
        issue([field], `仅接受精确 HTTPS 地址：${raw}`)
      }
      if (["code", "state", "error", "iss"].some((key) => url.searchParams.has(key))) {
        issue([field], `地址不能预先包含协议响应参数：${raw}`)
      }
    }
  }

  const resources = new Set<string>()
  const scopes = new Set<string>()
  value.permissions.forEach((permission, index) => {
    if (!RESOURCE_KEY.test(permission.resourceKey.trim())) issue(["permissions", index, "resourceKey"], "资源标识格式无效")
    const entries = splitScopes(permission.scopesText)
    if (!entries.length) issue(["permissions", index, "scopesText"], "至少选择一个 scope")
    if (new Set(entries).size !== entries.length) issue(["permissions", index, "scopesText"], "scope 不能重复")
    if (resources.has(permission.resourceKey.trim())) issue(["permissions", index, "resourceKey"], "资源不能重复")
    resources.add(permission.resourceKey.trim())
    entries.forEach((scope) => {
      if (!SCOPE_NAME.test(scope)) issue(["permissions", index, "scopesText"], `scope 格式无效：${scope}`)
      scopes.add(scope)
    })
  })
  if (value.subjectMode === "USER" && !scopes.has("openid")) {
    issue(["permissions"], "人员 OIDC 客户端必须包含 openid scope")
  }
  if (value.subjectMode === "MACHINE" && scopes.has("openid")) {
    issue(["permissions"], "机器客户端不申请 OIDC openid scope")
  }
})

type ClientFormValues = z.infer<typeof clientFormSchema>
type RecoveryAction = "create" | "update" | "rotate" | "revoke" | "retire"
type RecoveryState = { operationId: string; action: RecoveryAction; clientLabel: string; contextKey?: string; sessionId?: string; restored?: boolean; error?: string }
type FilterKey = "ownership" | "environment" | "clientType" | "subjectMode" | "status"

const defaultFormValues: ClientFormValues = {
  applicationId: "",
  environment: "test",
  displayName: "",
  subjectMode: "USER",
  runtimeKind: "WEB_BFF",
  contextMode: "ACCOUNT_ONLY",
  allowedMemberTypes: [],
  refreshToken: false,
  autoAuthorize: false,
  accessTokenTtlSeconds: "",
  refreshTokenTtlSeconds: "",
  clientTokenTtlSeconds: "",
  redirectUrisText: "",
  postLogoutRedirectUrisText: "",
  logoutMode: "SHORT_SESSION_ONLINE_CHECK",
  backchannelLogoutUri: "",
  backchannelTargetId: "",
  status: "DISABLED",
  consentGroupId: "",
  permissions: [{ resourceKey: "userinfo", scopesText: "openid, profile" }],
}

function splitLines(raw: string) { return raw.split(/\r?\n/).map((line) => line.trim()).filter(Boolean) }
function splitScopes(raw: string) { return raw.split(/[\s,]+/).map((scope) => scope.trim()).filter(Boolean) }
function toPageNumber(value: string | null, fallback: number) {
  const parsed = Number(value)
  return Number.isInteger(parsed) && parsed > 0 ? parsed : fallback
}
function ownershipLabel(value: string) { return value === "THIRD_PARTY" ? "第三方" : "自有" }
function subjectLabel(value: UnifiedSubjectMode) { return value === "MACHINE" ? "机器" : "人员" }
function runtimeLabel(value: UnifiedRuntimeKind) {
  return ({ WEB_BFF: "Web BFF", SPA: "浏览器应用", NATIVE: "原生应用", SERVICE: "服务" })[value]
}
function contextLabel(value: UnifiedContextMode) {
  return ({ ACCOUNT_ONLY: "纯账号", PLATFORM_CONTEXT: "业务上下文", NONE: "无用户上下文" })[value]
}
function formatDate(value: string) {
  const date = new Date(value)
  return Number.isNaN(date.valueOf()) ? "—" : new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(date)
}
function clientTypeFor(subject: UnifiedSubjectMode, runtime: UnifiedRuntimeKind) {
  return subject === "MACHINE" || runtime === "WEB_BFF" || runtime === "SERVICE" ? "CONFIDENTIAL" : "PUBLIC"
}
function tokenMethodFor(subject: UnifiedSubjectMode, runtime: UnifiedRuntimeKind) {
  return clientTypeFor(subject, runtime) === "CONFIDENTIAL" ? "client_secret_basic" : "none"
}
function payloadFromRecord(client: UnifiedClientRecord, status: UnifiedClientStatus): UnifiedClientPayload {
  return {
    expectedVersion: client.version,
    applicationId: client.applicationId,
    environment: client.environment,
    displayName: client.displayName,
    clientType: client.clientType,
    runtimeKind: client.runtimeKind,
    subjectMode: client.subjectMode,
    contextMode: client.contextMode,
    allowedMemberTypes: client.allowedMemberTypes,
    grantTypes: client.grantTypes,
    tokenEndpointAuthMethod: client.tokenEndpointAuthMethod,
    redirectUris: client.redirectUris,
    postLogoutRedirectUris: client.postLogoutRedirectUris,
    logoutMode: client.logoutMode ?? "SHORT_SESSION_ONLINE_CHECK",
    backchannelLogoutUri: client.backchannelLogoutUri ?? null,
    backchannelTargetId: client.backchannelTargetId ?? null,
    status,
    consentGroupId: client.consentGroupId,
    autoAuthorize: client.autoAuthorize,
    accessTokenTtlSeconds: client.accessTokenTtlSeconds,
    refreshTokenTtlSeconds: client.refreshTokenTtlSeconds,
    clientTokenTtlSeconds: client.clientTokenTtlSeconds,
    permissions: client.permissions.map(({ resourceKey, scopes }) => ({ resourceKey, scopes })),
  }
}

function formValuesFromClient(client: UnifiedClientRecord): ClientFormValues {
  return {
    applicationId: client.applicationId,
    environment: client.environment === "production" ? "production" : "test",
    displayName: client.displayName,
    subjectMode: client.subjectMode,
    runtimeKind: client.runtimeKind,
    contextMode: client.contextMode,
    allowedMemberTypes: [...client.allowedMemberTypes],
    refreshToken: client.grantTypes.includes("refresh_token"),
    autoAuthorize: client.autoAuthorize,
    accessTokenTtlSeconds: client.accessTokenTtlSeconds === null ? "" : String(client.accessTokenTtlSeconds),
    refreshTokenTtlSeconds: client.refreshTokenTtlSeconds === null ? "" : String(client.refreshTokenTtlSeconds),
    clientTokenTtlSeconds: client.clientTokenTtlSeconds === null ? "" : String(client.clientTokenTtlSeconds),
    redirectUrisText: client.redirectUris.join("\n"),
    postLogoutRedirectUrisText: client.postLogoutRedirectUris.join("\n"),
    logoutMode: client.logoutMode ?? "SHORT_SESSION_ONLINE_CHECK",
    backchannelLogoutUri: client.backchannelLogoutUri ?? "",
    backchannelTargetId: client.backchannelTargetId ?? "",
    status: client.status,
    consentGroupId: client.consentGroupId ?? "",
    permissions: client.permissions.map((permission) => ({
      resourceKey: permission.resourceKey,
      scopesText: permission.scopes.join(", "),
    })),
  }
}

export default function UnifiedClientsPage() {
  const queryClient = useQueryClient()
  const recoveryScope = useAuthStore((state) => `${state.contextKey ?? ""}:${state.sessionId ?? ""}`)
  const [searchParams, setSearchParams] = useSearchParams()
  const [applicationDraft, setApplicationDraft] = React.useState(searchParams.get("applicationId") ?? "")
  const applicationFilterRef = React.useRef<HTMLInputElement>(null)
  const [editorOpen, setEditorOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<UnifiedClientRecord | null>(null)
  const [detail, setDetail] = React.useState<UnifiedClientRecord | null>(null)
  const [rotateTarget, setRotateTarget] = React.useState<UnifiedClientRecord | null>(null)
  const [revokeTarget, setRevokeTarget] = React.useState<UnifiedClientRecord | null>(null)
  const [retireTarget, setRetireTarget] = React.useState<UnifiedClientRecord | null>(null)
  const [secret, setSecret] = React.useState<string | null>(null)
  const [secretTitle, setSecretTitle] = React.useState("客户端密钥")
  const [recovery, setRecovery] = React.useState<RecoveryState | null>(() => {
    try { const pending = pendingIdentityOperation(); return pending ? { ...pending, restored: true } : null } catch { return null }
  })
  const [writing, setWriting] = React.useState(false)
  const [versionConflict, setVersionConflict] = React.useState(false)
  const [conflictReloading, setConflictReloading] = React.useState(false)
  const writeLock = React.useRef(false)
  const restoredScope = React.useRef<string | null>(null)
  const editorOpener = React.useRef<HTMLElement | null>(null)

  const pageSize = React.useMemo(() => {
    const raw = Number(searchParams.get("size"))
    return PAGE_SIZES.includes(raw) ? raw : 10
  }, [searchParams])
  const maxPage = Math.floor(100_000 / pageSize) + 1
  const page = Math.min(toPageNumber(searchParams.get("page"), 1), maxPage)
  const filters = React.useMemo(() => ({
    applicationId: searchParams.get("applicationId") ?? "",
    ownership: searchParams.get("ownership") ?? ALL,
    environment: searchParams.get("environment") ?? ALL,
    clientType: searchParams.get("clientType") ?? ALL,
    subjectMode: searchParams.get("subjectMode") ?? ALL,
    status: searchParams.get("status") ?? ALL,
  }), [searchParams])

  React.useEffect(() => setApplicationDraft(filters.applicationId), [filters.applicationId])

  const updateSearch = React.useCallback((patch: Record<string, string>, resetPage = false) => {
    setSearchParams((previous) => {
      const next = new URLSearchParams(previous)
      for (const [key, value] of Object.entries(patch)) {
        if (!value || value === ALL) next.delete(key)
        else next.set(key, value)
      }
      if (resetPage) next.delete("page")
      return next
    }, { replace: true })
  }, [setSearchParams])

  const queryParams: UnifiedClientListParams = {
    applicationId: filters.applicationId || undefined,
    ownership: filters.ownership === ALL ? undefined : filters.ownership as UnifiedClientListParams["ownership"],
    environment: filters.environment === ALL ? undefined : filters.environment,
    clientType: filters.clientType === ALL ? undefined : filters.clientType as UnifiedClientListParams["clientType"],
    subjectMode: filters.subjectMode === ALL ? undefined : filters.subjectMode as UnifiedClientListParams["subjectMode"],
    status: filters.status === ALL ? undefined : filters.status as UnifiedClientListParams["status"],
    limit: pageSize,
    offset: (page - 1) * pageSize,
  }
  const clientsQuery = useQuery({
    queryKey: ["identity-clients", filters, page, pageSize],
    queryFn: () => identityClientsApi.list(queryParams),
    placeholderData: keepPreviousData,
    staleTime: 20_000,
    retry: 1,
  })
  const appsQuery = useQuery({
    queryKey: ["applications", "identity-client-suggestions"],
    queryFn: async () => asArray<ApplicationRecord>(await applicationsApi.list()),
    staleTime: 60_000,
    retry: false,
  })
  const clients = clientsQuery.data?.records ?? []
  const total = clientsQuery.data?.page.total ?? 0
  const totalPages = Math.max(1, Math.ceil(total / pageSize))

  React.useEffect(() => {
    if (clientsQuery.isSuccess && page > totalPages) updateSearch({ page: String(totalPages) })
  }, [clientsQuery.isSuccess, page, totalPages, updateSearch])

  const form = useForm<ClientFormValues>({
    resolver: zResolver(clientFormSchema),
    defaultValues: defaultFormValues,
  })
  const permissions = useFieldArray({ control: form.control, name: "permissions" })
  const subjectMode = form.watch("subjectMode")
  const runtimeKind = form.watch("runtimeKind")
  const contextMode = form.watch("contextMode")
  const logoutMode = form.watch("logoutMode")
  const selectedApplicationId = form.watch("applicationId")
  const watchedPermissions = form.watch("permissions")
  const machine = subjectMode === "MACHINE"
  const clientType = clientTypeFor(subjectMode, runtimeKind)

  const projectionQuery = useQuery({
    queryKey: ["identity-application-projection", selectedApplicationId],
    queryFn: () => identityClientsApi.applicationProjection(selectedApplicationId),
    enabled: editorOpen && isUuidV7(selectedApplicationId),
    staleTime: 15_000,
    retry: false,
  })
  const projection: ApplicationAuthProjection | undefined = projectionQuery.data
  const projectionScopes = projection?.approvedScopes ?? []
  const projectionBlocked = !editing && projection?.status !== undefined && projection.status !== "APPROVED"

  const openCreate = () => {
    if (writeLock.current || recovery) return
    editorOpener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
    setVersionConflict(false)
    setEditing(null)
    form.reset(defaultFormValues)
    setEditorOpen(true)
  }
  const openEdit = (client: UnifiedClientRecord) => {
    if (writeLock.current || recovery) return
    editorOpener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
    setVersionConflict(false)
    setEditing(client)
    form.reset(formValuesFromClient(client))
    setEditorOpen(true)
  }

  const reloadAfterVersionConflict = async () => {
    if (!editing || conflictReloading) return
    setConflictReloading(true)
    try {
      const latest = await identityClientsApi.get(editing.id)
      setEditing(latest)
      form.reset(formValuesFromClient(latest))
      setVersionConflict(false)
      toast.success(`已载入最新版本 v${latest.version}`)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "读取最新客户端配置失败")
    } finally {
      setConflictReloading(false)
    }
  }

  const closeSecret = (open: boolean) => {
    if (!open) setSecret(null)
  }

  const completeRecoveredOperation = React.useCallback(async (state: RecoveryState, operation: UnifiedClientOperation) => {
    if (state.contextKey && useAuthStore.getState().contextKey !== state.contextKey) return
    if (operation.operationId !== state.operationId) throw new Error("原操作结果的标识不匹配")
    if (operation.state === "NOT_APPLIED") {
      finishIdentityOperation(state.operationId, state.contextKey)
      setRecovery(null)
      if (!state.restored && (state.action === "create" || state.action === "update")) setEditorOpen(true)
      toast.warning("服务器确认该操作未执行，可安全地重新提交")
      return
    }
    if (operation.state !== "COMPLETED") throw new Error("原操作尚未返回可核验的终态")
    const result = operation.result as Record<string, unknown> | null
    if (!result) throw new Error("原操作返回的结果为空")
    finishIdentityOperation(state.operationId, state.contextKey)
    if (typeof result.clientSecret === "string") {
      setSecret(result.clientSecret)
      setSecretTitle("一次性客户端密钥")
      if (typeof result.previousCredentialExpiresAt === "string") {
        toast.info(`旧密钥将在 ${formatDate(result.previousCredentialExpiresAt)} 到期`)
      }
      if (typeof result.credentialExpiresAt === "string") toast.info(`当前密钥最晚于 ${formatDate(result.credentialExpiresAt)} 到期，请在此之前完成轮换`)
    }
    setRecovery(null)
    setEditorOpen(false)
    setEditing(null)
    setRotateTarget(null)
    setRevokeTarget(null)
    setRetireTarget(null)
    await queryClient.invalidateQueries({ queryKey: ["identity-clients"] })
    await queryClient.invalidateQueries({ queryKey: ["identity-consent-groups"] })
    toast.success(`${state.clientLabel}：原操作已完成`)
  }, [queryClient])

  const recoverOperation = React.useCallback(async (state: RecoveryState) => {
    if (state.contextKey && useAuthStore.getState().contextKey !== state.contextKey) return
    setRecovery({ ...state, error: undefined })
    try {
      const operation = await identityClientsApi.operation(state.operationId)
      await completeRecoveredOperation(state, operation)
    } catch (error) {
      if (error instanceof Error && error.name === "AbortError") return
      if (state.contextKey && useAuthStore.getState().contextKey !== state.contextKey) return
      setRecovery({
        ...state,
        error: error instanceof ApiError && [403, 404].includes(error.status)
          ? "当前登录会话不能读取此操作。请返回原会话查询；原会话无法恢复时，请将操作编号交给授权管理员核查执行结果，勿重复提交。"
          : error instanceof Error ? error.message : "结果仍未确认",
      })
    }
  }, [completeRecoveredOperation])

  React.useLayoutEffect(() => {
    if (restoredScope.current === recoveryScope) return
    restoredScope.current = recoveryScope
    setSecret(null)
    setEditorOpen(false)
    try {
      const pending = pendingIdentityOperation()
      const restored = pending ? { ...pending, restored: true } : null
      setRecovery(restored)
      if (restored) void recoverOperation(restored)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "无法读取管理操作恢复记录")
    }
  }, [recoveryScope, recoverOperation])

  const reportMutationError = React.useCallback(async (error: unknown, action: RecoveryAction, clientLabel: string) => {
    if (error instanceof ApiError && error.code.startsWith("OPERATION_STORAGE_")) {
      toast.error(error.message)
      return false
    }
    if (error instanceof ApiError && error.code === "OIDC_REMOTE_RESULT_UNKNOWN" && error.operationId) {
      const saved = pendingIdentityOperation()
      const pending = saved?.operationId === error.operationId ? saved : { operationId: error.operationId, action, clientLabel }
      setRecovery(pending)
      setEditorOpen(false)
      setRotateTarget(null)
      setRevokeTarget(null)
      setRetireTarget(null)
      toast.warning("操作结果尚未确认，正在查询原操作；请勿重复提交")
      await recoverOperation(pending)
      return true
    }
    if (error instanceof ApiError && error.status === 409) {
      await queryClient.invalidateQueries({ queryKey: ["identity-clients"] })
      await queryClient.invalidateQueries({ queryKey: ["identity-consent-groups"] })
      setVersionConflict(action === "update")
      toast.error("配置已被另一位管理员修改，请刷新后重新检查版本")
      return false
    }
    toast.error(error instanceof Error ? error.message : "操作失败")
    return false
  }, [queryClient, recoverOperation])

  const payloadFor = (values: ClientFormValues): UnifiedClientPayload => ({
    expectedVersion: editing?.version ?? null,
    applicationId: values.applicationId.trim().toLowerCase(),
    environment: values.environment,
    displayName: values.displayName.trim(),
    clientType,
    runtimeKind: values.runtimeKind,
    subjectMode: values.subjectMode,
    contextMode: values.subjectMode === "MACHINE" ? "NONE" : values.contextMode,
    allowedMemberTypes: values.contextMode === "PLATFORM_CONTEXT" && values.subjectMode === "USER" ? values.allowedMemberTypes : [],
    grantTypes: values.subjectMode === "MACHINE"
      ? ["client_credentials"]
      : ["authorization_code", ...(values.refreshToken ? ["refresh_token"] : [])],
    tokenEndpointAuthMethod: tokenMethodFor(values.subjectMode, values.runtimeKind),
    redirectUris: values.subjectMode === "MACHINE" ? [] : splitLines(values.redirectUrisText),
    postLogoutRedirectUris: values.subjectMode === "MACHINE" ? [] : splitLines(values.postLogoutRedirectUrisText),
    logoutMode: values.logoutMode,
    backchannelLogoutUri: values.backchannelLogoutUri || null,
    backchannelTargetId: values.backchannelTargetId || null,
    status: values.status,
    autoAuthorize: values.subjectMode === "USER" && values.autoAuthorize,
    accessTokenTtlSeconds: values.subjectMode === "USER" ? tokenSeconds(values.accessTokenTtlSeconds) : null,
    refreshTokenTtlSeconds: values.subjectMode === "USER" && values.refreshToken ? tokenSeconds(values.refreshTokenTtlSeconds, true) : null,
    clientTokenTtlSeconds: values.subjectMode === "MACHINE" ? tokenSeconds(values.clientTokenTtlSeconds) : null,
    consentGroupId: values.consentGroupId.trim() ? values.consentGroupId.trim().toLowerCase() : null,
    permissions: values.permissions.map((permission) => ({
      resourceKey: permission.resourceKey.trim(),
      scopes: splitScopes(permission.scopesText),
    })),
  })

  const onSave = async (values: ClientFormValues) => {
    if (writeLock.current || recovery) return
    if (!editing && values.logoutMode === "BACK_CHANNEL") { toast.error("先以短会话合同创建客户端，再完成owner目标核验后编辑back-channel"); return }
    if (projectionBlocked) {
      toast.error("应用尚未通过审核，不能登记或启用客户端")
      return
    }
    writeLock.current = true
    setWriting(true)
    try {
      const payload = payloadFor(values)
      if (editing) {
        await identityClientsApi.update(editing.id, payload)
        toast.success("客户端配置已更新")
      } else {
        await identityClientsApi.create(payload)
        toast.success("客户端已登记；机密客户端请继续轮换生成一次性 secret")
      }
      setEditorOpen(false)
      await queryClient.invalidateQueries({ queryKey: ["identity-clients"] })
      await queryClient.invalidateQueries({ queryKey: ["identity-consent-groups"] })
    } catch (error) {
      await reportMutationError(error, editing ? "update" : "create", values.displayName || "客户端")
    } finally {
      writeLock.current = false
      setWriting(false)
    }
  }

  const withWriteLock = async (run: () => Promise<void>) => {
    if (writeLock.current || recovery) return
    writeLock.current = true
    setWriting(true)
    try { await run() } finally { writeLock.current = false; setWriting(false) }
  }

  const rotate = async (client: UnifiedClientRecord) => withWriteLock(async () => {
    try {
      const result = await identityClientsApi.rotateCredential(client.id, client.version)
      setSecret(result.clientSecret)
      setSecretTitle("一次性客户端密钥")
      setRotateTarget(null)
      toast.success("新密钥已生成，请立即保存")
      if (result.previousCredentialExpiresAt) toast.info(`旧密钥将在 ${formatDate(result.previousCredentialExpiresAt)} 到期`)
      toast.info(`当前密钥最晚于 ${formatDate(result.credentialExpiresAt)} 到期，请在此之前完成轮换`)
      await queryClient.invalidateQueries({ queryKey: ["identity-clients"] })
    } catch (error) {
      await reportMutationError(error, "rotate", client.displayName)
    }
  })

  const revoke = async (client: UnifiedClientRecord) => withWriteLock(async () => {
    try {
      const result = await identityClientsApi.revokeCredentials(client.id, client.version)
      setRevokeTarget(null)
      toast.success(`已紧急撤销 ${result.revokedCredentials} 个凭据版本`)
      await queryClient.invalidateQueries({ queryKey: ["identity-clients"] })
    } catch (error) {
      await reportMutationError(error, "revoke", client.displayName)
    }
  })

  const retire = async (client: UnifiedClientRecord) => withWriteLock(async () => {
    try {
      await identityClientsApi.update(client.id, payloadFromRecord(client, "RETIRED"))
      setRetireTarget(null)
      toast.success("客户端已退役，无法恢复")
      await queryClient.invalidateQueries({ queryKey: ["identity-clients"] })
    } catch (error) {
      await reportMutationError(error, "retire", client.displayName)
    }
  })

  const changeFilter = (key: FilterKey, value: string) => updateSearch({ [key]: value }, true)
  const applyTextFilters = () => {
    if (applicationDraft.trim() && !isUuidV7(applicationDraft.trim())) {
      toast.error("请选择应用或输入有效的应用标识")
      return
    }
    updateSearch({ applicationId: applicationDraft.trim().toLowerCase() }, true)
  }
  const clearFilters = () => {
    setApplicationDraft("")
    setSearchParams((previous) => {
      const next = new URLSearchParams(previous)
      for (const key of ["applicationId", "partnerId", "ownership", "environment", "clientType", "subjectMode", "status", "page"]) next.delete(key)
      return next
    }, { replace: true })
  }
  const setPage = (nextPage: number) => updateSearch({ page: String(nextPage) })
  const setSize = (nextSize: number) => updateSearch({ size: String(nextSize), page: "1" })

  const renderClientActions = (client: UnifiedClientRecord) => (
    <div className="flex flex-wrap items-center gap-1.5">
      <Button variant="ghost" size="icon" className="size-8" aria-label={`查看 ${client.displayName} 配置`} onClick={() => setDetail(client)}>
        <Eye className="size-4" />
      </Button>
      <Button variant="ghost" size="icon" className="size-8" aria-label={`编辑 ${client.displayName}`} disabled={writing || !!recovery || client.status === "RETIRED"} onClick={() => openEdit(client)}>
        <ChevronDown className="size-4 rotate-[-90deg]" />
      </Button>
      {client.clientType === "CONFIDENTIAL" && client.status !== "RETIRED" && (
        <Button variant="outline" size="sm" className="h-8 gap-1.5 px-2" aria-label={`轮换 ${client.displayName} 密钥`} disabled={writing || !!recovery} onClick={() => setRotateTarget(client)}>
          <KeyRound className="size-3.5" />
          <span className="hidden xl:inline">轮换</span>
        </Button>
      )}
      {client.clientType === "CONFIDENTIAL" && client.status !== "RETIRED" && (
        <Button variant="ghost" size="icon" className="size-8 text-destructive hover:text-destructive" aria-label={`撤销 ${client.displayName} 凭据`} disabled={writing || !!recovery} onClick={() => setRevokeTarget(client)}>
          <ShieldAlert className="size-4" />
        </Button>
      )}
      {client.status !== "RETIRED" && (
        <Button variant="ghost" size="icon" className="size-8 text-muted-foreground" aria-label={`退役 ${client.displayName}`} disabled={writing || !!recovery} onClick={() => setRetireTarget(client)}>
          <Trash2 className="size-4" />
        </Button>
      )}
    </div>
  )

  return (
    <div className="space-y-5 pb-8">
      <section className="relative isolate overflow-hidden rounded-2xl bg-[#143a36] px-5 py-6 text-white shadow-sm sm:px-7 sm:py-7">
        <div aria-hidden className="pointer-events-none absolute inset-0 opacity-40" style={{ backgroundImage: "radial-gradient(circle at 82% 10%, rgba(180, 220, 176, .34), transparent 28%), linear-gradient(135deg, transparent 49.8%, rgba(210, 232, 214, .08) 50%, transparent 50.2%)", backgroundSize: "auto, 24px 24px" }} />
        <div aria-hidden className="pointer-events-none absolute -right-8 -top-14 font-mono text-[10rem] font-semibold leading-none text-white/5 sm:text-[13rem]">04</div>
        <div className="relative flex flex-col gap-6 sm:flex-row sm:items-end sm:justify-between">
          <div className="max-w-2xl">
            <div className="mb-4 flex flex-wrap items-center gap-2">
              <span className="inline-flex items-center gap-2 rounded-full border border-emerald-100/20 bg-emerald-50/10 px-3 py-1 text-[11px] font-semibold tracking-[0.16em] text-emerald-50">
                <span className="size-1.5 rounded-full bg-[#b8d79c] shadow-[0_0_12px_rgba(184,215,156,.9)]" />
                CLIENT MANAGEMENT
              </span>
              <span className="font-mono text-[10px] uppercase tracking-[0.2em] text-white/50">authority 01</span>
            </div>
            <h1 className="text-2xl font-semibold tracking-tight sm:text-3xl">客户端管理</h1>
            <p className="mt-2 max-w-xl text-sm leading-6 text-emerald-50/70">管理客户端、登录回调、授权范围与访问凭据。</p>
          </div>
          <div className="flex items-end justify-between gap-6 sm:justify-end">
            <div>
              <p className="font-mono text-[10px] uppercase tracking-[0.18em] text-white/50">登记条目</p>
              <p className="mt-1 font-mono text-3xl font-medium tabular-nums">{clientsQuery.data?.page.total ?? "—"}</p>
            </div>
            <div>
              <p className="font-mono text-[10px] uppercase tracking-[0.18em] text-white/50">本页机密客户端</p>
              <p className="mt-1 font-mono text-3xl font-medium tabular-nums">{clients.filter((client) => client.clientType === "CONFIDENTIAL").length}</p>
            </div>
            <Button onClick={openCreate} disabled={writing || !!recovery} className="h-10 shrink-0 bg-[#d2e7c0] text-[#12332f] hover:bg-[#e1efd5]">
              <Plus className="size-4" />新增客户端
            </Button>
          </div>
        </div>
      </section>

      {recovery && (
        <div role="status" className="flex flex-col gap-3 rounded-xl border border-amber-500/35 bg-amber-500/10 p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0">
            <p className="flex items-center gap-2 text-sm font-semibold"><AlertCircle className="size-4 text-amber-600" />{recovery.clientLabel} 的结果仍待确认</p>
            <p className="mt-1 text-xs text-muted-foreground">请查询原操作，不要重新提交。操作编号 <code className="break-all font-mono">{recovery.operationId}</code></p>
            {recovery.error && <p className="mt-1 text-xs text-destructive">查询失败：{recovery.error}</p>}
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <CopyButton text={recovery.operationId} size="sm" label="复制编号" />
            <Button size="sm" onClick={() => void recoverOperation(recovery)} disabled={writing}><RefreshCw className="size-3.5" />查询原操作</Button>
          </div>
        </div>
      )}

      <section className="rounded-xl border bg-card shadow-sm">
        <div className="flex flex-col gap-3 border-b px-4 py-4 sm:flex-row sm:items-center sm:justify-between sm:px-5">
          <div className="flex items-center gap-2">
            <div className="flex size-8 items-center justify-center rounded-lg bg-primary/10 text-primary"><Search className="size-4" /></div>
            <div>
              <h2 className="text-sm font-semibold">筛选客户端</h2>
              <p className="text-xs text-muted-foreground">筛选与页码保存在当前地址中</p>
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" onClick={() => void clientsQuery.refetch()} disabled={clientsQuery.isFetching}>
              <RefreshCw className={`size-3.5 ${clientsQuery.isFetching ? "animate-spin" : ""}`} />刷新
            </Button>
            <Button variant="ghost" size="sm" onClick={clearFilters}>清空筛选</Button>
          </div>
        </div>
        <div className="grid gap-3 p-4 sm:grid-cols-2 lg:grid-cols-4 xl:grid-cols-7 sm:px-5">
          <FilterSelect label="归属" value={filters.ownership} onChange={(value) => changeFilter("ownership", value)} options={[[ALL, "全部归属"], ["FIRST_PARTY", "自有应用"], ["THIRD_PARTY", "第三方应用"]]} />
          <FilterSelect label="环境" value={filters.environment} onChange={(value) => changeFilter("environment", value)} options={[[ALL, "全部环境"], ["test", "测试"], ["production", "生产"]]} />
          <FilterSelect label="客户端类型" value={filters.clientType} onChange={(value) => changeFilter("clientType", value)} options={[[ALL, "全部类型"], ["CONFIDENTIAL", "机密"], ["PUBLIC", "公开"]]} />
          <FilterSelect label="主体" value={filters.subjectMode} onChange={(value) => changeFilter("subjectMode", value)} options={[[ALL, "全部主体"], ["USER", "人员"], ["MACHINE", "机器"]]} />
          <FilterSelect label="状态" value={filters.status} onChange={(value) => changeFilter("status", value)} options={[[ALL, "全部状态"], ["ENABLED", "启用"], ["DISABLED", "停用"], ["RETIRED", "已退役"]]} />
          <div className="space-y-1.5">
            <Label htmlFor="filter-application" className="text-[11px] font-medium text-muted-foreground">应用</Label>
            <div className="relative">
              <Input ref={applicationFilterRef} id="filter-application" list="identity-client-filter-apps" value={applicationDraft} onChange={(event) => {
                const value = event.target.value
                setApplicationDraft(value)
                if (!value.trim() || isUuidV7(value.trim())) updateSearch({ applicationId: value.trim().toLowerCase() }, true)
              }} onKeyDown={(event) => { if (event.key === "Enter") applyTextFilters() }} placeholder="选择应用或输入应用标识" className={`h-9 pr-10 font-mono text-xs ${applicationDraft ? "[&::-webkit-calendar-picker-indicator]:hidden" : ""}`} />
              {applicationDraft && <Button type="button" variant="ghost" size="icon" className="absolute top-1/2 right-1 size-7 -translate-y-1/2 text-muted-foreground" aria-label="清除应用筛选" onClick={() => {
                setApplicationDraft("")
                updateSearch({ applicationId: "" }, true)
                applicationFilterRef.current?.focus()
              }}><X className="size-3.5" /></Button>}
            </div>
            <datalist id="identity-client-filter-apps">{(appsQuery.data ?? []).map((app) => <option key={app.id} value={app.id}>{app.name}</option>)}</datalist>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2 border-t bg-muted/20 px-4 py-2.5 sm:px-5">
          <span className="font-mono text-[10px] uppercase tracking-[0.15em] text-muted-foreground">Active filters</span>
          {Object.entries(filters).filter(([, value]) => value && value !== ALL).length === 0 ? (
            <span className="text-xs text-muted-foreground">无筛选</span>
          ) : Object.entries(filters).filter(([, value]) => value && value !== ALL).map(([key, value]) => (
            <Badge key={key} variant="outline" className="max-w-full gap-1.5 font-mono text-[10px]">
              <span className="truncate">{key === "ownership" ? ownershipLabel(value) : `${key}: ${value}`}</span>
              <button type="button" className="rounded-full hover:text-destructive" aria-label={`移除 ${key} 筛选`} onClick={() => updateSearch({ [key]: "" }, true)}><X className="size-3" /></button>
            </Badge>
          ))}
          {clientsQuery.isFetching && <span className="ml-auto inline-flex items-center gap-1.5 text-xs text-muted-foreground"><Loader2 className="size-3.5 animate-spin" />同步中</span>}
        </div>
      </section>

      {clientsQuery.isError && !clientsQuery.data && (
        <ErrorState message={clientsQuery.error instanceof Error ? clientsQuery.error.message : undefined} onRetry={() => void clientsQuery.refetch()} />
      )}
      {clientsQuery.isError && clientsQuery.data && (
        <div role="status" className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-warning/35 bg-warning/10 px-4 py-3 text-sm">
          <span className="flex items-center gap-2"><AlertCircle className="size-4 text-warning" />刷新失败，仍显示上次成功读取的配置</span>
          <Button variant="outline" size="sm" onClick={() => void clientsQuery.refetch()}>重试读取</Button>
        </div>
      )}

      <section className="overflow-hidden rounded-xl border bg-card shadow-sm">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3.5 sm:px-5">
          <div className="flex items-center gap-2">
            <Layers3 className="size-4 text-primary" />
            <h2 className="text-sm font-semibold">客户端目录</h2>
            {clientsQuery.data && <Badge variant="secondary" className="font-mono text-[10px]">{clientsQuery.data.page.total} 条</Badge>}
          </div>
          <div className="flex items-center gap-2 text-xs text-muted-foreground"><ArrowDownUp className="size-3.5" />按登记时间与 UUID 稳定排序</div>
        </div>
        {clientsQuery.isLoading && !clientsQuery.data ? (
          <div className="p-3"><TableSkeleton cols={7} /></div>
        ) : clients.length === 0 ? (
          <EmptyState
            title={Object.values(filters).some((value) => value !== ALL && value !== "") ? "没有匹配的客户端" : "注册簿尚无客户端"}
            description="统一登记自有平台、客户门户、第三方应用与机器接入；创建后再按需生成机密客户端凭据。"
            icon={<KeyRound className="size-6" />}
            action={<Button size="sm" onClick={openCreate} disabled={!!recovery}><Plus className="size-4" />登记第一条客户端</Button>}
          />
        ) : (
          <>
            <div className="hidden overflow-x-auto md:block">
              <Table className="min-w-[1050px]">
                <TableHeader>
                  <TableRow className="bg-muted/30 hover:bg-muted/30">
                    <TableHead className="pl-5">CLIENT / 客户端</TableHead>
                    <TableHead>APPLICATION / 应用</TableHead>
                    <TableHead>ENV</TableHead>
                    <TableHead>CONTRACT / 类型</TableHead>
                    <TableHead>PERMISSIONS / 权限</TableHead>
                    <TableHead>STATUS / 版本</TableHead>
                    <TableHead className="pr-5 text-right">动作</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {clients.map((client) => <ClientTableRow key={client.id} client={client} actions={renderClientActions(client)} onView={() => setDetail(client)} />)}
                </TableBody>
              </Table>
            </div>
            <div className="space-y-3 p-3 md:hidden">
              {clients.map((client) => <ClientMobileCard key={client.id} client={client} actions={renderClientActions(client)} onView={() => setDetail(client)} />)}
            </div>
          </>
        )}
      </section>

      {clientsQuery.data && <DataPagination page={page} size={pageSize} total={total} onPageChange={setPage} onSizeChange={setSize} />}

      <Dialog open={editorOpen} onOpenChange={(open) => { if (!open && !writing && !recovery) { setEditorOpen(false); setEditing(null) } }}>
        <DialogContent className="flex max-h-[calc(100dvh-1rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-4xl" onCloseAutoFocus={(event) => {
          // Several external buttons open this controlled dialog without a Radix trigger.
          const opener = editorOpener.current
          editorOpener.current = null
          if (opener?.isConnected && !opener.hasAttribute("disabled")) {
            event.preventDefault()
            opener.focus()
          }
        }}>
          <DialogHeader className="shrink-0 border-b bg-background px-5 py-4 pr-12 text-left sm:px-6">
            <div className="flex items-center gap-2 text-[10px] font-semibold uppercase tracking-[0.18em] text-primary"><Fingerprint className="size-3.5" /> Identity registry / client record</div>
            <DialogTitle className="mt-1 text-xl">{editing ? "编辑客户端登记" : "登记新客户端"}</DialogTitle>
            <DialogDescription>{editing ? `版本 ${editing.version} · 应用、环境与主体模式固定；权限修改会推进策略代际。` : "所有字段都会由身份服务重新校验；创建不生成或展示密钥。"}</DialogDescription>
          </DialogHeader>
          <form onSubmit={form.handleSubmit(onSave)} className="flex min-h-0 flex-1 flex-col">
            <div className="min-h-0 flex-1 space-y-5 overflow-y-auto overscroll-contain px-5 py-5 sm:px-6">
              {versionConflict && editing && (
                <div role="alert" className="flex flex-col gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 sm:flex-row sm:items-center sm:justify-between">
                  <p className="text-xs leading-5">此记录已被其他管理员更新。当前草稿尚未覆盖；载入最新版本会放弃本次未保存修改。</p>
                  <Button type="button" size="sm" variant="outline" className="shrink-0" onClick={() => void reloadAfterVersionConflict()} disabled={conflictReloading}>
                    {conflictReloading ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
                    载入最新版本
                  </Button>
                </div>
              )}
              <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_260px]">
                <div className="space-y-5">
                  <section className="grid gap-4 rounded-xl border bg-muted/15 p-4 sm:grid-cols-2">
                    <FormField label="应用" error={form.formState.errors.applicationId?.message} hint="选择应用，也可粘贴应用标识。">
                      <div className="relative">
                        <Input aria-label="应用" list="identity-client-apps" className={`pr-10 font-mono text-xs ${selectedApplicationId ? "[&::-webkit-calendar-picker-indicator]:hidden" : ""}`} placeholder="选择应用或输入应用标识" disabled={!!editing} {...form.register("applicationId")} />
                        {!editing && selectedApplicationId && <Button type="button" variant="ghost" size="icon" className="absolute top-1/2 right-1 size-7 -translate-y-1/2 text-muted-foreground" aria-label="清除应用选择" onClick={() => {
                          form.setValue("applicationId", "", { shouldDirty: true })
                          form.clearErrors("applicationId")
                          form.setFocus("applicationId")
                        }}><X className="size-3.5" /></Button>}
                      </div>
                      <datalist id="identity-client-apps">{(appsQuery.data ?? []).map((app) => <option key={app.id} value={app.id}>{app.name}</option>)}</datalist>
                    </FormField>
                    <FormField label="显示名称" error={form.formState.errors.displayName?.message}>
                      <Input maxLength={120} placeholder="示例 CRM Web" {...form.register("displayName")} />
                    </FormField>
                    <FormField label="运行环境" error={form.formState.errors.environment?.message}>
                      <Select value={form.watch("environment")} onValueChange={(value) => form.setValue("environment", value as ClientFormValues["environment"], { shouldDirty: true })} disabled={!!editing}>
                        <SelectTrigger aria-label="运行环境"><SelectValue /></SelectTrigger>
                        <SelectContent><SelectItem value="test">测试 / test</SelectItem><SelectItem value="production">生产 / production</SelectItem></SelectContent>
                      </Select>
                    </FormField>
                    <FormField label="客户端状态" error={form.formState.errors.status?.message} hint="退役是终态，请使用目录行操作。">
                      <Select value={form.watch("status")} onValueChange={(value) => form.setValue("status", value as UnifiedClientStatus, { shouldDirty: true })}>
                        <SelectTrigger aria-label="客户端状态"><SelectValue /></SelectTrigger>
                        <SelectContent><SelectItem value="DISABLED">停用</SelectItem><SelectItem value="ENABLED">启用</SelectItem></SelectContent>
                      </Select>
                    </FormField>
                  </section>

                  <section className="space-y-4 rounded-xl border p-4">
                    <div className="flex items-center gap-2"><ShieldCheck className="size-4 text-primary" /><h3 className="text-sm font-semibold">主体与运行形态</h3></div>
                    <div className="grid gap-4 sm:grid-cols-2">
                      <FormField label="主体模式" error={form.formState.errors.subjectMode?.message}>
                        <Select value={subjectMode} disabled={!!editing} onValueChange={(value) => {
                          const next = value as UnifiedSubjectMode
                          form.setValue("subjectMode", next, { shouldDirty: true })
                          form.setValue("runtimeKind", next === "MACHINE" ? "SERVICE" : "WEB_BFF", { shouldDirty: true })
                          form.setValue("contextMode", next === "MACHINE" ? "NONE" : "ACCOUNT_ONLY", { shouldDirty: true })
                          form.setValue("allowedMemberTypes", [], { shouldDirty: true })
                          form.setValue("refreshToken", false, { shouldDirty: true })
                          form.setValue("autoAuthorize", false, { shouldDirty: true })
                          form.setValue("accessTokenTtlSeconds", "", { shouldDirty: true })
                          form.setValue("refreshTokenTtlSeconds", "", { shouldDirty: true })
                          form.setValue("clientTokenTtlSeconds", "", { shouldDirty: true })
                          form.setValue("logoutMode", "SHORT_SESSION_ONLINE_CHECK", { shouldDirty: true })
                          form.setValue("backchannelLogoutUri", "", { shouldDirty: true })
                          form.setValue("backchannelTargetId", "", { shouldDirty: true })
                          form.setValue("permissions", next === "MACHINE"
                            ? [{ resourceKey: "", scopesText: "" }]
                            : defaultFormValues.permissions.map((permission) => ({ ...permission })), { shouldDirty: true })
                          if (next === "MACHINE") { form.setValue("redirectUrisText", ""); form.setValue("postLogoutRedirectUrisText", "") }
                        }}>
                          <SelectTrigger aria-label="主体模式"><SelectValue /></SelectTrigger>
                          <SelectContent><SelectItem value="USER">人员 USER</SelectItem><SelectItem value="MACHINE">机器 MACHINE</SelectItem></SelectContent>
                        </Select>
                      </FormField>
                      <FormField label="运行类型" error={form.formState.errors.runtimeKind?.message}>
                        <Select value={runtimeKind} disabled={!!editing} onValueChange={(value) => form.setValue("runtimeKind", value as UnifiedRuntimeKind, { shouldDirty: true })}>
                          <SelectTrigger aria-label="运行类型"><SelectValue /></SelectTrigger>
                          <SelectContent>
                            <SelectItem value="WEB_BFF" disabled={machine}>WEB_BFF · 服务端 Web</SelectItem>
                            <SelectItem value="SPA" disabled={machine || contextMode === "PLATFORM_CONTEXT"}>SPA · 浏览器应用</SelectItem>
                            <SelectItem value="NATIVE" disabled={machine || contextMode === "PLATFORM_CONTEXT"}>NATIVE · 原生应用</SelectItem>
                            <SelectItem value="SERVICE" disabled={!machine}>SERVICE · 服务</SelectItem>
                          </SelectContent>
                        </Select>
                      </FormField>
                      <FormField label="客户端类型" hint="由运行类型推导，不能单独改写。">
                        <div className="flex h-10 items-center gap-2 rounded-md border bg-muted/35 px-3 text-sm">
                          <Badge variant={clientType === "CONFIDENTIAL" ? "default" : "outline"}>{clientType === "CONFIDENTIAL" ? "CONFIDENTIAL · 机密" : "PUBLIC · 公开"}</Badge>
                          <span className="text-xs text-muted-foreground">{tokenMethodFor(subjectMode, runtimeKind)}</span>
                        </div>
                      </FormField>
                      <FormField label="登录上下文" error={form.formState.errors.contextMode?.message}>
                        <Select value={contextMode} disabled={!!editing || machine} onValueChange={(value) => {
                          const next = value as UnifiedContextMode
                          form.setValue("contextMode", next, { shouldDirty: true })
                          if (next === "PLATFORM_CONTEXT") form.setValue("runtimeKind", "WEB_BFF", { shouldDirty: true, shouldValidate: true })
                          if (next === "ACCOUNT_ONLY") form.setValue("allowedMemberTypes", [], { shouldDirty: true })
                        }}>
                          <SelectTrigger aria-label="登录上下文"><SelectValue /></SelectTrigger>
                          <SelectContent>
                            <SelectItem value="ACCOUNT_ONLY" disabled={machine}>ACCOUNT_ONLY · 纯账号</SelectItem>
                            <SelectItem value="PLATFORM_CONTEXT" disabled={machine}>PLATFORM_CONTEXT · 业务身份</SelectItem>
                            <SelectItem value="NONE" disabled={!machine}>NONE · 无用户上下文</SelectItem>
                          </SelectContent>
                        </Select>
                      </FormField>
                    </div>
                    {contextMode === "PLATFORM_CONTEXT" && !machine && (
                      <FormField label="允许的成员类型" error={form.formState.errors.allowedMemberTypes?.message} hint="选择一种员工或客户身份；两类会话使用独立客户端。">
                        <div className="flex flex-wrap gap-3 rounded-lg bg-muted/25 p-3">
                          {MEMBER_TYPES.map((member) => (
                            <label key={member} className="flex cursor-pointer items-center gap-2 text-sm">
                              <Checkbox checked={form.watch("allowedMemberTypes").includes(member)} disabled={!!editing} onCheckedChange={(checked) => {
                                form.setValue("allowedMemberTypes", checked === true ? [member] : [], { shouldDirty: true, shouldValidate: true })
                              }} />{member === "EMPLOYEE" ? "员工" : "客户"}
                            </label>
                          ))}
                        </div>
                      </FormField>
                    )}
                  </section>

                  <ClientAuthorizationSettings machine={machine} editing={!!editing}
                    firstParty={(editing?.ownership ?? projection?.ownership) === "FIRST_PARTY"}
                    value={form.watch()} defaults={editing?.tokenDefaults}
                    errors={{ accessTokenTtlSeconds: form.formState.errors.accessTokenTtlSeconds?.message,
                      refreshTokenTtlSeconds: form.formState.errors.refreshTokenTtlSeconds?.message,
                      clientTokenTtlSeconds: form.formState.errors.clientTokenTtlSeconds?.message }}
                    onChange={(field, value) => {
                      if (field === "autoAuthorize" || field === "refreshToken") form.setValue(field, value === true, { shouldDirty: true, shouldValidate: true })
                      else form.setValue(field, String(value), { shouldDirty: true, shouldValidate: true })
                    }} />

                  {!machine && (
                    <section className="grid gap-4 rounded-xl border p-4 sm:grid-cols-2">
                      <FormField label="回调地址 · 每行一个" error={form.formState.errors.redirectUrisText?.message}>
                        <Textarea className="min-h-24 font-mono text-xs" placeholder="https://client.example.com/callback" {...form.register("redirectUrisText")} />
                      </FormField>
                      <FormField label="退出后返回地址 · 每行一个" error={form.formState.errors.postLogoutRedirectUrisText?.message}>
                        <Textarea className="min-h-24 font-mono text-xs" placeholder="https://client.example.com/signed-out" {...form.register("postLogoutRedirectUrisText")} />
                      </FormField>
                    </section>
                  )}

                  {!machine && <section className="space-y-4 rounded-xl border p-4">
                    <div><h3 className="text-sm font-semibold">登出与会话失效</h3><p className="mt-1 text-xs leading-5 text-muted-foreground">新客户端先使用短会话与在线检查。启用back-channel前，须由owner为此客户端核验目标；第三方地址还须与当前环境的独立审核清单一致。</p></div>
                    <FormField label="登出合同" error={form.formState.errors.logoutMode?.message}>
                      <Select value={logoutMode} disabled={!editing || clientType !== "CONFIDENTIAL"} onValueChange={value => { form.setValue("logoutMode", value as ClientFormValues["logoutMode"], { shouldDirty: true }); if (value === "SHORT_SESSION_ONLINE_CHECK") { form.setValue("backchannelLogoutUri", "", { shouldDirty: true }); form.setValue("backchannelTargetId", "", { shouldDirty: true }) } }}>
                        <SelectTrigger aria-label="客户端登出合同"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="SHORT_SESSION_ONLINE_CHECK">短会话与在线检查</SelectItem><SelectItem value="BACK_CHANNEL">已审核 back-channel</SelectItem></SelectContent>
                      </Select>
                    </FormField>
                    {logoutMode === "BACK_CHANNEL" && <div className="grid gap-4 sm:grid-cols-2"><FormField label="已审核 back-channel HTTPS 地址" error={form.formState.errors.backchannelLogoutUri?.message}><Input className="font-mono text-xs" aria-label="客户端back-channel地址" {...form.register("backchannelLogoutUri")} /></FormField><FormField label="owner核验目标引用 UUIDv7" error={form.formState.errors.backchannelTargetId?.message}><Input className="font-mono text-xs" aria-label="客户端back-channel目标引用" {...form.register("backchannelTargetId")} /></FormField></div>}
                    <p className="text-xs leading-5 text-muted-foreground">目标引用只用于查找已有核验事实。填写UUID不会建立核验目标，也不保证已投递；缺失、过期或不匹配时服务端拒绝启用。停用或退役保留原登出配置。</p>
                  </section>}

                  <section className="space-y-4 rounded-xl border p-4">
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div>
                        <div className="flex items-center gap-2"><Layers3 className="size-4 text-primary" /><h3 className="text-sm font-semibold">资源与 scope 权限</h3></div>
                        <p className="mt-1 text-xs text-muted-foreground">只能授予应用已批准且资源目录仍启用的 scope。</p>
                      </div>
                      <Button type="button" variant="outline" size="sm" disabled={permissions.fields.length >= 64} onClick={() => permissions.append({ resourceKey: "", scopesText: "" })}><Plus className="size-3.5" />添加资源</Button>
                    </div>
                    {projection && (
                      <div className={`flex items-start gap-2 rounded-lg px-3 py-2 text-xs ${projection.status === "APPROVED" ? "bg-primary/8 text-foreground" : "bg-warning/10 text-warning"}`}>
                        {projection.status === "APPROVED" ? <BadgeCheck className="mt-0.5 size-3.5 shrink-0 text-primary" /> : <AlertCircle className="mt-0.5 size-3.5 shrink-0" />}
                        <span>{projection.displayName} · {ownershipLabel(projection.ownership)} · {projection.status} · 当前批准 {projectionScopes.length} 个 scope</span>
                      </div>
                    )}
                    {projectionQuery.isError && <p className="text-xs text-warning">无法读取应用批准范围；可继续录入，最终仍由身份服务强制校验。</p>}
                    <div className="space-y-3">
                      {permissions.fields.map((field, index) => {
                        const currentScopes = splitScopes(watchedPermissions[index]?.scopesText ?? "")
                        return (
                          <div key={field.id} className="space-y-3 rounded-lg border bg-muted/15 p-3">
                            <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end">
                              <FormField label={`资源 ${String(index + 1).padStart(2, "0")}`} error={form.formState.errors.permissions?.[index]?.resourceKey?.message}>
                                <Input className="font-mono text-xs" placeholder="userinfo" {...form.register(`permissions.${index}.resourceKey`)} />
                              </FormField>
                              <Button type="button" variant="ghost" size="icon" className="size-9 text-muted-foreground hover:text-destructive" aria-label="移除资源权限" disabled={permissions.fields.length <= 1} onClick={() => permissions.remove(index)}><Trash2 className="size-4" /></Button>
                            </div>
                            <FormField label="Scope 名称 · 逗号或空格分隔" error={form.formState.errors.permissions?.[index]?.scopesText?.message}>
                              <Input className="font-mono text-xs" placeholder="openid, profile" {...form.register(`permissions.${index}.scopesText`)} />
                            </FormField>
                            {projectionScopes.length > 0 && (
                              <div className="flex flex-wrap gap-1.5">
                                {projectionScopes.map((scope) => {
                                  const chosen = currentScopes.includes(scope)
                                  return <button key={scope} type="button" onClick={() => {
                                    const next = chosen ? currentScopes.filter((item) => item !== scope) : [...currentScopes, scope]
                                    form.setValue(`permissions.${index}.scopesText`, next.join(", "), { shouldDirty: true, shouldValidate: true })
                                  }} className={`rounded-full border px-2.5 py-1 font-mono text-[10px] transition-colors ${chosen ? "border-primary/35 bg-primary/10 text-primary" : "border-border bg-background text-muted-foreground hover:border-primary/40"}`} aria-pressed={chosen}>{chosen && <Check className="mr-1 inline size-3" />}{scope}</button>
                                })}
                              </div>
                            )}
                          </div>
                        )
                      })}
                    </div>
                  </section>
                </div>

                <aside className="space-y-3">
                  <div className="rounded-xl border bg-[#eef5ef] p-4 dark:bg-emerald-950/25">
                    <div className="flex items-center gap-2 text-primary"><Sparkles className="size-4" /><p className="text-xs font-semibold uppercase tracking-[0.14em]">形态预览</p></div>
                    <p className="mt-3 text-lg font-semibold">{clientType === "CONFIDENTIAL" ? "CONFIDENTIAL" : "PUBLIC"}</p>
                    <p className="mt-1 text-sm text-muted-foreground">{subjectLabel(subjectMode)} · {runtimeLabel(runtimeKind)}</p>
                    <p className="mt-3 border-t border-primary/15 pt-3 text-xs leading-5 text-muted-foreground">{contextLabel(contextMode)}；{machine ? "使用机器凭据，不签发 ID Token 或 refresh token。" : "浏览器与服务端依运行类型使用不同的客户端认证方式。"}</p>
                  </div>
                  <div className="rounded-xl border p-4">
                    <div className="flex items-center gap-2"><CircleDot className="size-4 text-primary" /><p className="text-sm font-semibold">登记约束</p></div>
                    <ul className="mt-3 space-y-2 text-xs leading-5 text-muted-foreground">
                      <li>• 应用必须已批准；scope 只能取自应用批准集合。</li>
                      <li>• 应用、环境与主体不能在编辑时转换。</li>
                      <li>• secret 只在凭据轮换后一次显示，不会保存进此页面状态。</li>
                    </ul>
                  </div>
                  {editing && (
                    <div className="rounded-xl border bg-muted/20 p-4 text-xs text-muted-foreground">
                      <p className="font-mono uppercase tracking-[0.12em]">Expected version</p>
                      <p className="mt-1 font-mono text-xl text-foreground">v{editing.version}</p>
                      <p className="mt-1">保存会推进 policyVersion 并使旧授权按服务端版本校验失效。</p>
                    </div>
                  )}
                </aside>
              </div>
            </div>
            <DialogFooter className="shrink-0 border-t bg-background px-5 py-3 sm:px-6">
              <Button type="button" variant="ghost" disabled={writing} onClick={() => { setEditorOpen(false); setEditing(null) }}>取消</Button>
              <Button type="submit" disabled={writing || !!recovery || projectionBlocked || versionConflict}>
                {writing ? <Loader2 className="size-4 animate-spin" /> : editing ? <ClipboardCheck className="size-4" /> : <Plus className="size-4" />}
                {editing ? "保存更改" : "创建客户端"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={!!detail} onOpenChange={(open) => { if (!open) setDetail(null) }}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader><DialogTitle>{detail?.displayName ?? "客户端详情"}</DialogTitle><DialogDescription>读取自统一身份客户端登记</DialogDescription></DialogHeader>
          {detail && <ClientDetail client={detail} />}
        </DialogContent>
      </Dialog>

      <ConfirmDialog open={!!rotateTarget} onOpenChange={(open) => { if (!open && !writing) setRotateTarget(null) }} title="轮换客户端凭据？" confirmText="生成新密钥" loading={writing} onConfirm={() => rotateTarget && void rotate(rotateTarget)} description={rotateTarget && <span>为 <strong>{rotateTarget.displayName}</strong> 生成一份新密钥。旧密钥最多保留 24 小时用于平滑切换；本次 secret 只显示一次。</span>} />
      <ConfirmDialog open={!!revokeTarget} onOpenChange={(open) => { if (!open && !writing) setRevokeTarget(null) }} title="紧急撤销全部凭据？" confirmText="立即撤销" loading={writing} onConfirm={() => revokeTarget && void revoke(revokeTarget)} description={revokeTarget && <span><strong>{revokeTarget.displayName}</strong> 的当前与重叠凭据会立即失效，相关 client policyVersion 会递增。</span>} />
      <ConfirmDialog open={!!retireTarget} onOpenChange={(open) => { if (!open && !writing) setRetireTarget(null) }} title="永久退役客户端？" confirmText="退役客户端" loading={writing} onConfirm={() => retireTarget && void retire(retireTarget)} description={retireTarget && <span><strong>{retireTarget.displayName}</strong> 退役后不能恢复；应用和环境绑定仍留在审计记录中。</span>} />
      {secret && <SecretDialog open onOpenChange={closeSecret} secret={secret} title={secretTitle} />}
    </div>
  )
}

function FilterSelect({ label, value, onChange, options }: { label: string; value: string; onChange: (value: string) => void; options: [string, string][] }) {
  return <div className="space-y-1.5"><Label className="text-[11px] font-medium text-muted-foreground">{label}</Label><Select value={value} onValueChange={onChange}><SelectTrigger aria-label={label} className="h-9"><SelectValue /></SelectTrigger><SelectContent>{options.map(([option, title]) => <SelectItem key={option} value={option}>{title}</SelectItem>)}</SelectContent></Select></div>
}

function FormField({ label, error, hint, children }: { label: string; error?: string; hint?: string; children: React.ReactNode }) {
  return <div className="space-y-1.5"><Label className="text-xs font-medium">{label}</Label>{children}{error ? <p className="text-xs text-destructive">{error}</p> : hint ? <p className="text-[11px] leading-4 text-muted-foreground">{hint}</p> : null}</div>
}

function ClientTableRow({ client, actions, onView }: { client: UnifiedClientRecord; actions: React.ReactNode; onView: () => void }) {
  return <TableRow className="group">
    <TableCell className="max-w-[18rem] pl-5">
      <button type="button" onClick={onView} className="block max-w-full text-left font-medium hover:text-primary"><span className="block truncate">{client.displayName}</span></button>
      <div className="mt-1 flex items-center gap-1.5"><code className="truncate font-mono text-[10px] text-muted-foreground">{client.clientId}</code><CopyButton text={client.clientId} size="icon" className="size-6 shrink-0 border-0 p-0 text-muted-foreground" /></div>
    </TableCell>
    <TableCell className="max-w-[15rem]">
      <div className="flex items-center gap-1.5"><Building2 className="size-3.5 shrink-0 text-muted-foreground" /><span className="truncate text-sm">{client.applicationName}</span><Badge variant="outline" className="shrink-0 text-[9px]">{ownershipLabel(client.ownership)}</Badge></div>
      <code className="mt-1 block truncate font-mono text-[10px] text-muted-foreground">{client.applicationId}</code>
    </TableCell>
    <TableCell><Badge variant="secondary" className="font-mono text-[10px]">{client.environment}</Badge></TableCell>
    <TableCell className="min-w-[10rem]">
      <div className="flex flex-wrap gap-1"><Badge variant={client.clientType === "CONFIDENTIAL" ? "default" : "outline"} className="text-[10px]">{client.clientType === "CONFIDENTIAL" ? "机密" : "公开"}</Badge><Badge variant="outline" className="text-[10px]">{subjectLabel(client.subjectMode)}</Badge></div>
      <p className="mt-1 text-[10px] text-muted-foreground">{runtimeLabel(client.runtimeKind)} · {contextLabel(client.contextMode)}</p>
    </TableCell>
    <TableCell className="max-w-[16rem]">
      <div className="flex flex-wrap gap-1">{client.permissions.flatMap((permission) => permission.scopes.map((scope) => <Badge key={`${permission.resourceKey}:${scope}`} variant="outline" className="font-mono text-[9px]">{scope}</Badge>))}</div>
      <p className="mt-1 truncate font-mono text-[10px] text-muted-foreground">{client.permissions.map((permission) => permission.resourceKey).join(" · ")}</p>
    </TableCell>
    <TableCell><StatusBadge status={client.status} /><p className="mt-1 font-mono text-[10px] text-muted-foreground">v{client.version} / policy {client.policyVersion}</p></TableCell>
    <TableCell className="pr-5 text-right">{actions}</TableCell>
  </TableRow>
}

function ClientMobileCard({ client, actions, onView }: { client: UnifiedClientRecord; actions: React.ReactNode; onView: () => void }) {
  return <article className="rounded-xl border bg-background p-4 shadow-xs">
    <div className="flex items-start justify-between gap-3"><button type="button" className="min-w-0 text-left" onClick={onView}><span className="block truncate text-sm font-semibold">{client.displayName}</span><code className="mt-1 block break-all font-mono text-[10px] text-muted-foreground">{client.clientId}</code></button><StatusBadge status={client.status} /></div>
    <div className="mt-3 grid grid-cols-2 gap-3 border-y py-3 text-xs">
      <div><p className="text-[10px] text-muted-foreground">应用</p><p className="mt-0.5 truncate font-medium">{client.applicationName}</p><p className="font-mono text-[9px] text-muted-foreground">{ownershipLabel(client.ownership)} · {client.environment}</p></div>
      <div><p className="text-[10px] text-muted-foreground">客户端合约</p><p className="mt-0.5 font-medium">{client.clientType} · {subjectLabel(client.subjectMode)}</p><p className="text-[10px] text-muted-foreground">{runtimeLabel(client.runtimeKind)} · {contextLabel(client.contextMode)}</p></div>
    </div>
    <div className="mt-3 flex flex-wrap gap-1">{client.permissions.flatMap((permission) => permission.scopes.map((scope) => <Badge key={`${permission.resourceKey}:${scope}`} variant="outline" className="font-mono text-[9px]">{scope}</Badge>))}</div>
    <div className="mt-3 flex flex-wrap items-center justify-between gap-2"><span className="font-mono text-[10px] text-muted-foreground">v{client.version} / policy {client.policyVersion}</span>{actions}</div>
  </article>
}

function ClientDetail({ client }: { client: UnifiedClientRecord }) {
  return <div className="max-h-[65dvh] space-y-4 overflow-y-auto pr-1 text-sm">
    <div className="grid gap-3 sm:grid-cols-2"><Detail label="所有者" value={`${ownershipLabel(client.ownership)} · ${client.applicationName}`} /><Detail label="环境" value={client.environment} /><Detail label="主体 / 运行" value={`${subjectLabel(client.subjectMode)} · ${runtimeLabel(client.runtimeKind)}`} /><Detail label="上下文" value={contextLabel(client.contextMode)} /><Detail label="版本" value={`client v${client.version} · policy v${client.policyVersion}`} /><Detail label="Token 认证" value={client.tokenEndpointAuthMethod} /></div>
    <div className="grid gap-3 sm:grid-cols-2">
      <Detail label="授权确认" value={client.subjectMode === "MACHINE" ? "服务间调用，无用户授权" : client.autoAuthorize ? "自动授权" : "用户确认授权"} />
      <Detail label={client.subjectMode === "MACHINE" ? "客户端令牌有效期" : "访问令牌有效期"}
        value={`${client.subjectMode === "MACHINE" ? client.clientTokenTtlSeconds ?? client.tokenDefaults.clientTokenTtlSeconds : client.accessTokenTtlSeconds ?? client.tokenDefaults.accessTokenTtlSeconds} 秒（受原来源期限限制）`} />
      {client.grantTypes.includes("refresh_token") && <Detail label="刷新令牌" value={`${client.refreshTokenTtlSeconds ?? client.tokenDefaults.refreshTokenTtlSeconds} 秒空闲有效期 · 每次轮换`} />}
    </div>
    <Detail label="应用 UUID" value={client.applicationId} mono />
    <div className="rounded-lg border p-3"><p className="text-xs font-medium text-muted-foreground">回调地址</p>{client.redirectUris.length ? client.redirectUris.map((uri) => <p key={uri} className="mt-1 break-all font-mono text-xs">{uri}</p>) : <p className="mt-1 text-xs">无（机器客户端）</p>}</div>
    <div className="rounded-lg border p-3"><p className="text-xs font-medium text-muted-foreground">退出后返回地址</p>{client.postLogoutRedirectUris.length ? client.postLogoutRedirectUris.map((uri) => <p key={uri} className="mt-1 break-all font-mono text-xs">{uri}</p>) : <p className="mt-1 text-xs">未登记</p>}</div>
    <div className="grid gap-3 rounded-lg border p-3 sm:grid-cols-2"><Detail label="登出合同" value={client.logoutMode === "BACK_CHANNEL" ? "已绑定 back-channel" : "短会话与在线检查"} />{client.backchannelLogoutUri && <Detail label="Back-channel HTTPS 地址" value={client.backchannelLogoutUri} />}{client.backchannelTargetId && <Detail label="owner核验目标引用" value={client.backchannelTargetId} />}</div>
    <div className="space-y-2">{client.permissions.map((permission) => <div key={permission.resourceKey} className="rounded-lg border p-3"><div className="flex flex-wrap justify-between gap-2"><code className="font-mono text-xs">{permission.resourceKey}</code><span className="break-all font-mono text-[10px] text-muted-foreground">{permission.audience}</span></div><div className="mt-2 flex flex-wrap gap-1">{permission.scopes.map((scope) => <Badge key={scope} variant="outline" className="font-mono text-[10px]">{scope}</Badge>)}</div></div>)}</div>
    <p className="text-[10px] text-muted-foreground">登记时间 {formatDate(client.createdAt)} · 最近更新 {formatDate(client.updatedAt)}</p>
  </div>
}

function Detail({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return <div className="min-w-0 rounded-lg border bg-muted/15 p-3"><p className="text-[10px] text-muted-foreground">{label}</p><p className={`mt-1 break-all text-xs ${mono ? "font-mono" : "font-medium"}`}>{value}</p></div>
}
