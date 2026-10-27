import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import { Bot, KeyRound, LayoutGrid, Loader2, Pencil, Plus, RefreshCw, ShieldCheck, Trash2, Unlink, Unlock } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Badge } from "@/components/ui/badge"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { PageHeader } from "@/components/layout/PageHeader"
import { StatusBadge } from "@/components/StatusBadge"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { CopyButton } from "@/components/CopyButton"
import { SecretDialog } from "@/components/CopyButton"
import { DataPagination } from "@/components/DataPagination"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { OidcClientsPanel } from "./OidcClientsPanel"
import { TableSkeleton, EmptyState, ErrorState } from "@/components/states"
import { oauthApi } from "@/api/oauth"
import { applicationsApi } from "@/api/applications"
import { admissionMemberLabels, admissionSubjectLabels, type AdmissionPolicySummary, type AdmissionPolicyTarget } from "@/api/admission-policies"
import { AdmissionPolicyDialog } from "@/components/admission-policy-dialog"
import { useAuthStore } from "@/stores/auth"
import { asArray, asPage } from "@/lib/api"
import { GRANT_TYPE_LABELS, formatNumber } from "@/lib/format"
import { toast } from "sonner"
import type { ApplicationRecord, OAuthClientRecord } from "@/api/types"

const GRANT_TYPES = [
  "authorization_code",
  "refresh_token",
  "client_credentials",
  "urn:ietf:params:oauth:grant-type:device_code",
] as const
const SCOPES = ["profile", "email", "phone", "roles", "read", "write"] as const

const clientSchema = z.object({
  clientName: z.string().min(1, "请输入客户端名称").max(64),
  clientId: z
    .string()
    .max(64)
    .regex(/^[a-z0-9-]*$/, "仅支持小写字母、数字与中划线")
    .optional(),
  clientType: z.enum(["CONFIDENTIAL", "PUBLIC"]),
  requirePkce: z.boolean(),
  redirectUris: z.string(),
  subjectMode: z.enum(["USER", "MACHINE", "LEGACY_MIXED"]),
  allowedMemberTypes: z.array(z.enum(["EMPLOYEE", "CUSTOMER"])),
  grantTypes: z.array(z.string()).min(1, "至少选择一种授权类型"),
  scopes: z.array(z.string()),
  accessTokenValidity: z.coerce.number().int().min(60).max(86400),
  refreshTokenValidity: z.coerce.number().int().min(60).max(2592000),
  description: z.string().max(255).optional(),
  /* 关联平台应用: "none" = 不关联, 其余为应用 id */
  applicationId: z.string().optional(),
}).superRefine((value, context) => {
  const issue = (path: string, message: string) => context.addIssue({ code: "custom", path: [path], message })
  if (value.subjectMode === "MACHINE") {
    if (value.clientType !== "CONFIDENTIAL" || value.grantTypes.length !== 1 || value.grantTypes[0] !== "client_credentials") issue("grantTypes", "机器客户端须为机密类型，且仅使用客户端凭证授权")
    if (value.allowedMemberTypes.length) issue("allowedMemberTypes", "机器客户端不配置人员身份")
  } else {
    if (!value.allowedMemberTypes.length) issue("allowedMemberTypes", "至少允许一种成员身份")
    if (!value.redirectUris.trim()) issue("redirectUris", "至少填写一个回调地址")
    if (value.subjectMode === "USER" && value.grantTypes.includes("client_credentials")) issue("grantTypes", "人员客户端不能使用机器客户端凭证授权")
  }
})
type ClientForm = z.infer<typeof clientSchema>
type ClientWithPolicy = OAuthClientRecord & AdmissionPolicySummary

/** 「不关联」哨兵值(Radix Select 不允许空串 value) */
const NO_APP = "none"

/** 与后端 generate_client_id 对齐:非 ASCII 字母数字替换为 '-',空则回退 client */
function slugify(name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 32)
  return slug || "client"
}

export default function OAuthClientsPage() {
  const queryClient = useQueryClient()
  const [clientKind, setClientKind] = React.useState("oauth")
  const [page, setPage] = React.useState(1)
  const [size, setSize] = React.useState(10)

  const listQuery = useQuery({
    queryKey: ["oauth-clients", page, size],
    queryFn: async () => asPage<ClientWithPolicy>(await oauthApi.clients({ page, size }), page, size),
    staleTime: 30_000,
    retry: 1,
  })

  /* 关联应用候选(与菜单/授权页同源 applications query) */
  const appsQuery = useQuery({
    queryKey: ["applications"],
    queryFn: async () => asArray<ApplicationRecord>(await applicationsApi.list()),
    staleTime: 30_000,
    retry: 1,
  })
  const apps = appsQuery.data ?? []

  const [dialogOpen, setDialogOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<ClientWithPolicy | null>(null)
  const [policyTarget, setPolicyTarget] = React.useState<AdmissionPolicyTarget | null>(null)
  const openPolicy = (client: ClientWithPolicy) => setPolicyTarget({ kind: "client", id: client.id, name: client.clientName, fixedEmployee: client.clientId === "platform-console-legacy" })
  const [secret, setSecret] = React.useState<string | null>(null)
  const writing = React.useRef(false)
  const [writeBusy, setWriteBusy] = React.useState(false)
  const mounted = React.useRef(true)
  React.useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  const startWrite = () => {
    if (writing.current || !mounted.current) return false
    writing.current = true
    setWriteBusy(true)
    return true
  }
  const finishWrite = () => { writing.current = false; if (mounted.current) setWriteBusy(false) }

  const form = useForm<ClientForm>({
    resolver: zResolver(clientSchema),
    defaultValues: {
      clientName: "",
      clientId: "",
      clientType: "CONFIDENTIAL",
      requirePkce: false,
      redirectUris: "",
      subjectMode: "USER",
      allowedMemberTypes: ["EMPLOYEE"],
      grantTypes: ["authorization_code"],
      scopes: ["profile"],
      accessTokenValidity: 7200,
      refreshTokenValidity: 604800,
      description: "",
      applicationId: NO_APP,
    },
  })
  const clientName = form.watch("clientName")
  const subjectMode = form.watch("subjectMode")
  const machine = subjectMode === "MACHINE"

  const openCreate = () => {
    if (writing.current) return
    setEditing(null)
    form.reset({
      clientName: "",
      clientId: "",
      clientType: "CONFIDENTIAL",
      requirePkce: false,
      redirectUris: "",
      subjectMode: "USER",
      allowedMemberTypes: ["EMPLOYEE"],
      grantTypes: ["authorization_code"],
      scopes: ["profile"],
      accessTokenValidity: 7200,
      refreshTokenValidity: 604800,
      description: "",
      applicationId: NO_APP,
    })
    setDialogOpen(true)
  }

  const openEdit = (c: ClientWithPolicy) => {
    if (writing.current) return
    if (!c.subjectMode || !Array.isArray(c.allowedMemberTypes)) { toast.error("客户端准入信息未加载，请刷新列表后重试"); return }
    setEditing(c)
    form.reset({
      clientName: c.clientName,
      clientId: c.clientId,
      clientType: c.clientType,
      requirePkce: c.requirePkce ?? false,
      redirectUris: (c.redirectUris ?? []).join("\n"),
      subjectMode: c.subjectMode,
      allowedMemberTypes: [...c.allowedMemberTypes],
      grantTypes: c.grantTypes ?? [],
      scopes: c.scopes ?? [],
      accessTokenValidity: c.accessTokenValidity ?? 7200,
      refreshTokenValidity: c.refreshTokenValidity ?? 604800,
      description: c.description ?? "",
      applicationId: c.applicationId ?? NO_APP,
    })
    setDialogOpen(true)
  }

  const saveMutation = useMutation({
    mutationFn: async (v: ClientForm) => {
      const payload = {
        clientName: v.clientName,
        clientId: v.clientId,
        clientType: v.clientType,
        requirePkce: v.requirePkce,
        redirectUris: v.redirectUris
          .split(/\n+/)
          .map((s) => s.trim())
          .filter(Boolean),
        grantTypes: v.grantTypes,
        scopes: v.scopes,
        accessTokenValidity: v.accessTokenValidity,
        refreshTokenValidity: v.refreshTokenValidity,
        description: v.description || "",
        /* "none"/缺省 → null = 不关联/清除关联 */
        applicationId: v.applicationId && v.applicationId !== NO_APP ? v.applicationId : null,
        // Existing policies use the separate versioned endpoint. Creation has
        // no old version and always states its purpose and safe initial set.
        ...(!editing ? { subjectMode: v.subjectMode, allowedMemberTypes: v.subjectMode === "MACHINE" ? [] : v.allowedMemberTypes } : {}),
      }
      if (editing) {
        await oauthApi.updateClient(editing.id, payload)
        return { created: false, secret: null }
      }
      const resp = await oauthApi.createClient(payload)
      return { created: true, secret: resp.clientSecret ?? null }
    },
    onSuccess: (result) => {
      finishWrite()
      queryClient.invalidateQueries({ queryKey: ["oauth-clients"] })
      if (!mounted.current) return
      setDialogOpen(false)
      if (result.secret) {
        setSecret(result.secret)
        toast.success("客户端创建成功,请立即保存密钥")
      } else {
        toast.success(result.created ? "客户端创建成功" : "客户端已更新")
      }
    },
    onError: (e: Error) => { finishWrite(); if (mounted.current && e.name !== "AbortError") toast.error(e.message || "保存失败") },
  })

  const regenerateMutation = useMutation({
    mutationFn: (id: string) => oauthApi.regenerateSecret(id),
    onSuccess: (resp) => {
      finishWrite()
      queryClient.invalidateQueries({ queryKey: ["oauth-clients"] })
      if (!mounted.current) return
      setSecret(resp.clientSecret)
      toast.success("已重新生成密钥")
    },
    onError: (e: Error) => { finishWrite(); if (mounted.current && e.name !== "AbortError") toast.error(e.message || "重新生成失败") },
  })

  const [deleting, setDeleting] = React.useState<OAuthClientRecord | null>(null)
  const deleteMutation = useMutation({
    mutationFn: (id: string) => oauthApi.removeClient(id),
    onSuccess: () => {
      finishWrite()
      queryClient.invalidateQueries({ queryKey: ["oauth-clients"] })
      if (!mounted.current) return
      toast.success("客户端已删除")
      setDeleting(null)
    },
    onError: (e: Error) => { finishWrite(); if (mounted.current && e.name !== "AbortError") toast.error(e.message || "删除失败") },
  })

  const clients = listQuery.data?.records ?? []
  const total = listQuery.data?.total ?? 0
  React.useEffect(() => {
    if (listQuery.isSuccess && page > Math.max(1, Math.ceil(total / size))) setPage(Math.max(1, Math.ceil(total / size)))
  }, [listQuery.isSuccess, page, size, total])

  return (
    <div>
      <PageHeader
        title="OAuth 客户端"
        description="查看业务应用的 OAuth 接入与管理平台、客户门户的统一登录客户端"
        icon={<KeyRound className="size-5" />}
        actions={
          clientKind === "oauth" ? <Button onClick={openCreate} disabled={writeBusy}>
            <Plus className="size-4" />
            新建客户端
          </Button> : undefined
        }
      />

      <Tabs value={clientKind} onValueChange={setClientKind}>
        <TabsList aria-label="客户端类型">
          <TabsTrigger value="oauth">业务 OAuth 客户端</TabsTrigger>
          <TabsTrigger value="oidc">统一登录 OIDC 客户端</TabsTrigger>
        </TabsList>
        <TabsContent value="oauth" className="mt-4">
      <div className="rounded-xl border bg-card">
        {listQuery.isLoading ? (
          <div className="p-2">
            <TableSkeleton cols={9} />
          </div>
        ) : listQuery.isError ? (
          <ErrorState
            className="m-2 border-0 bg-transparent"
            message={listQuery.error instanceof Error ? listQuery.error.message : undefined}
            onRetry={() => listQuery.refetch()}
          />
        ) : clients.length === 0 ? (
          <EmptyState
            title="还没有 OAuth 客户端"
            description="注册客户端后即可通过标准 OAuth 2.0 流程接入平台"
            action={
              <Button size="sm" onClick={openCreate}>
                <Plus className="size-4" />
                新建客户端
              </Button>
            }
            icon={<KeyRound className="size-6" />}
          />
        ) : (
          <div className="table-scroll max-h-[32rem] overflow-auto scroll-area">
          <Table>
            <TableHeader className="sticky top-0 z-10 bg-card">
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-5 text-xs font-medium uppercase tracking-wider text-muted-foreground">客户端</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">关联应用</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">类型</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">身份准入</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">授权类型</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">Scopes</TableHead>
                <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">令牌有效期</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">状态</TableHead>
                <TableHead className="pr-5 text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {clients.map((c) => (
                <TableRow key={c.id}>
                  <TableCell className="pl-5">
                    <div className="flex items-center gap-2">
                      <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs font-medium">{c.clientId}</code>
                      <CopyButton text={c.clientId} size="icon" className="size-7 border-0 bg-transparent p-0 hover:bg-muted" />
                    </div>
                    <p className="mt-1 max-w-[16rem] truncate text-sm font-medium" title={c.clientName}>{c.clientName}</p>
                  </TableCell>
                  <TableCell className="text-sm">
                    {c.applicationName ? (
                      <span
                        className="block max-w-[10rem] truncate"
                        title={`${c.applicationName}${c.applicationId ? ` (${c.applicationId})` : ""}`}
                      >
                        {c.applicationName}
                      </span>
                    ) : (
                      <span className="text-xs text-muted-foreground">—</span>
                    )}
                  </TableCell>
                  <TableCell>
                    <Badge
                      variant="outline"
                      className={
                        c.clientType === "CONFIDENTIAL"
                          ? "gap-1 border-primary/30 bg-primary/10 text-primary"
                          : "gap-1 border-warning/40 bg-warning/10 text-warning"
                      }
                    >
                      {c.clientType === "CONFIDENTIAL" ? <ShieldCheck className="size-3" /> : <Unlock className="size-3" />}
                      {c.clientType === "CONFIDENTIAL" ? "机密" : "公开"}
                    </Badge>
                    <p className="mt-1 text-xs text-muted-foreground">{c.requirePkce ? "PKCE · 强制 S256" : "PKCE · 兼容模式"}</p>
                  </TableCell>
                  <TableCell>
                    <Button variant="ghost" size="sm" className="h-auto flex-col items-start gap-1.5 px-2 py-2" disabled={writeBusy} onClick={() => openPolicy(c)} aria-label={`管理 ${c.clientName} 的身份准入`}>
                      <span className="flex items-center gap-1.5 text-xs font-medium">{c.subjectMode === "MACHINE" ? <Bot className="size-3.5" /> : <ShieldCheck className="size-3.5" />}
                        {c.subjectMode ? admissionSubjectLabels[c.subjectMode] : "读取准入策略"}</span>
                      <span className="flex flex-wrap gap-1">{c.subjectMode === "MACHINE" ? <span className="text-[11px] text-muted-foreground">无人员身份</span> :
                        c.allowedMemberTypes?.map(type => <Badge key={type} variant="outline" className="text-[10px]">{admissionMemberLabels[type]}</Badge>)}</span>
                    </Button>
                  </TableCell>
                  <TableCell>
                    <div className="flex max-w-[16rem] flex-wrap gap-1">
                      {(c.grantTypes ?? []).map((g) => (
                        <Tooltip key={g}>
                          <TooltipTrigger asChild>
                            <Badge variant="secondary" className="max-w-[7.5rem] truncate text-[10px] font-normal">
                              {g === "urn:ietf:params:oauth:grant-type:device_code" ? "device_code" : g}
                            </Badge>
                          </TooltipTrigger>
                          <TooltipContent>{GRANT_TYPE_LABELS[g] ?? g}</TooltipContent>
                        </Tooltip>
                      ))}
                    </div>
                  </TableCell>
                  <TableCell>
                    <div className="flex max-w-[12rem] flex-wrap gap-1">
                      {(c.scopes ?? []).length === 0 ? (
                        <span className="text-xs text-muted-foreground">—</span>
                      ) : (
                        (c.scopes ?? []).map((s) => (
                          <Badge key={s} variant="outline" className="text-[10px] font-normal">
                            {s}
                          </Badge>
                        ))
                      )}
                    </div>
                  </TableCell>
                  <TableCell className="text-right text-xs text-muted-foreground tabular-nums">
                    <p>访问 {formatDuration(c.accessTokenValidity)}</p>
                    <p>刷新 {formatDuration(c.refreshTokenValidity)}</p>
                  </TableCell>
                  <TableCell>
                    <StatusBadge status={c.status} />
                  </TableCell>
                  <TableCell className="pr-5">
                    <div className="flex items-center justify-end gap-0.5">
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button variant="ghost" size="icon" className="size-8" aria-label="重新生成 Secret" onClick={() => { if (startWrite()) regenerateMutation.mutate(c.id) }} disabled={writeBusy}>
                            {regenerateMutation.isPending && regenerateMutation.variables === c.id ? (
                              <Loader2 className="size-4 animate-spin" />
                            ) : (
                              <RefreshCw className="size-4" />
                            )}
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>重新生成 Secret</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button variant="ghost" size="icon" className="size-8" aria-label="编辑" onClick={() => openEdit(c)}>
                            <Pencil className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>编辑</TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button variant="ghost" size="icon" className="size-8 text-destructive hover:text-destructive" aria-label="删除" disabled={writeBusy} onClick={() => setDeleting(c)}>
                            <Trash2 className="size-4" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>删除</TooltipContent>
                      </Tooltip>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          </div>
        )}
      </div>
      {listQuery.isSuccess && <DataPagination className="mt-4" page={page} size={size} total={total} onPageChange={setPage} onSizeChange={value => { setSize(value); setPage(1) }} />}
        </TabsContent>
        <TabsContent value="oidc" className="mt-4"><OidcClientsPanel /></TabsContent>
      </Tabs>

      {/* 新建/编辑 */}
      <Dialog open={dialogOpen} onOpenChange={value => { if (!writing.current) setDialogOpen(value) }}>
        <DialogContent className="grid-rows-[auto_minmax(0,1fr)_auto] overflow-hidden sm:max-w-xl">
          <DialogHeader className="pr-6">
            <DialogTitle>{editing ? `编辑客户端「${editing.clientName}」` : "新建 OAuth 客户端"}</DialogTitle>
            <DialogDescription>
              {editing ? "client_id 与客户端类型创建后不可变更；可调整 PKCE 策略" : "保存后自动分配 client_id,并生成一次性展示的 client_secret"}
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form id="client-form" onSubmit={form.handleSubmit((v) => { if (startWrite()) saveMutation.mutate(v) })} className="-mx-1 min-h-0 space-y-4 overflow-y-auto overscroll-contain px-1">
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="clientName"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        客户端名称 <span className="text-destructive">*</span>
                      </FormLabel>
                      <FormControl>
                        <Input placeholder="如: 智能电视" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="clientId"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>client_id</FormLabel>
                      <FormControl>
                        <Input
                          value={editing ? (field.value ?? "") : slugify(clientName) ? `${slugify(clientName)}-xxxxxx` : ""}
                          onChange={field.onChange}
                          className="font-mono"
                          disabled

                          placeholder={editing ? "" : "保存时自动生成"}
                        />
                      </FormControl>
                      <FormDescription>
                        {editing ? "创建后不可修改" : "由平台根据客户端名称自动生成唯一 ID"}
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              <FormField control={form.control} name="subjectMode" render={({ field }) => <FormItem>
                <FormLabel>主体用途</FormLabel>
                <Select value={field.value} disabled={!!editing || writeBusy} onValueChange={value => {
                  field.onChange(value)
                  form.setValue("allowedMemberTypes", value === "MACHINE" ? [] : ["EMPLOYEE"], { shouldDirty: true })
                  form.setValue("grantTypes", value === "MACHINE" ? ["client_credentials"] : ["authorization_code"], { shouldDirty: true })
                  if (value === "MACHINE") { form.setValue("clientType", "CONFIDENTIAL"); form.setValue("requirePkce", false) }
                }}>
                  <FormControl><SelectTrigger className="w-full"><SelectValue /></SelectTrigger></FormControl>
                  <SelectContent><SelectItem value="USER">人员登录 · USER</SelectItem><SelectItem value="MACHINE">机器凭据 · MACHINE</SelectItem>
                    {editing?.subjectMode === "LEGACY_MIXED" && <SelectItem value="LEGACY_MIXED" disabled>历史混合用途 · 仅维护原注册</SelectItem>}</SelectContent>
                </Select>
                <FormDescription>主体用途与公开／机密类型分别管理。新人员客户端默认仅员工；机器客户端不提供人员准入。</FormDescription><FormMessage />
              </FormItem>} />
              {machine ? <div className="flex items-start gap-3 rounded-lg border bg-muted/30 p-3 text-sm">
                <Bot className="mt-0.5 size-4 shrink-0 text-muted-foreground" /><p>机器客户端仅使用 CONFIDENTIAL 与 client_credentials。成员集合为空，不能作为员工或客户登录入口。</p>
              </div> : <FormField control={form.control} name="allowedMemberTypes" render={({ field }) => <FormItem>
                <FormLabel>允许的成员身份</FormLabel>
                <div className="grid gap-2 rounded-lg border p-3 sm:grid-cols-2">{(["EMPLOYEE", "CUSTOMER"] as const).map(type =>
                  <label key={type} className="flex items-center gap-2 text-sm"><Checkbox checked={field.value.includes(type)} disabled={!!editing || writeBusy}
                    onCheckedChange={checked => field.onChange(checked === true ? [...field.value.filter(value => value !== type), type] : field.value.filter(value => value !== type))} />{admissionMemberLabels[type]}</label>)}</div>
                {editing ? <Button type="button" variant="link" size="sm" className="h-auto px-0" disabled={writeBusy} onClick={() => { setDialogOpen(false); openPolicy(editing) }}>在身份准入中编辑集合与影响</Button> :
                  <FormDescription>客户用途须与目标应用的准入集合匹配，选择身份不会自动创建成员关系。</FormDescription>}<FormMessage />
              </FormItem>} />}

              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="clientType"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        客户端类型 <span className="text-destructive">*</span>
                      </FormLabel>
                      <Select onValueChange={value => { field.onChange(value); form.setValue("requirePkce", value === "PUBLIC", { shouldDirty: true }) }} value={field.value} disabled={!!editing || machine || writeBusy}>
                        <FormControl>
                          <SelectTrigger className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          <SelectItem value="CONFIDENTIAL">
                            <span className="flex items-center gap-2">
                              <ShieldCheck className="size-3.5 text-primary" />
                              CONFIDENTIAL(机密 · 服务端)
                            </span>
                          </SelectItem>
                          <SelectItem value="PUBLIC">
                            <span className="flex items-center gap-2">
                              <Unlock className="size-3.5 text-warning" />
                              PUBLIC(公开 · 设备/SPA)
                            </span>
                          </SelectItem>
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                {/* 关联应用(21-e 后端契约): null = 不关联 */}
                <FormField
                  control={form.control}
                  name="applicationId"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>关联应用</FormLabel>
                      <Select onValueChange={field.onChange} value={field.value ?? NO_APP}>
                        <FormControl>
                          <SelectTrigger className="w-full">
                            <SelectValue placeholder="选择要接入的平台应用" />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          <SelectItem value={NO_APP}>
                            <span className="flex items-center gap-2 text-muted-foreground">
                              <Unlink className="size-3.5" />
                              不关联
                            </span>
                          </SelectItem>
                          {apps.map((a) => (
                            <SelectItem key={a.id} value={a.id}>
                              <span className="flex items-center gap-2">
                                <LayoutGrid className="size-3.5 text-primary" />
                                {a.name}
                                <span className="font-mono text-[10px] text-muted-foreground">{a.code}</span>
                              </span>
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <FormDescription>关联后可在应用维度统一管理该客户端</FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              <FormField
                control={form.control}
                name="requirePkce"
                render={({ field }) => (
                  <FormItem className="rounded-lg border p-3">
                    <label className="flex items-center gap-2 text-sm font-medium">
                      <Checkbox aria-label="强制 PKCE S256" checked={field.value} disabled={machine || writeBusy} onCheckedChange={value => field.onChange(value === true)} />
                      授权码强制使用 PKCE（S256）
                    </label>
                    <FormDescription>启用后，授权请求必须携带 S256 code_challenge，兑换授权码时必须提交对应的 code_verifier。新公开客户端默认启用；已有接入方请先完成 S256 配置再开启，旧的无 PKCE 或 plain 请求需重新发起。关闭时保留兼容模式。仅影响授权码流程。</FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="redirectUris"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      回调地址 {!machine && <span className="text-destructive">*</span>}
                    </FormLabel>
                    <FormControl>
                      <Textarea rows={3} placeholder={"每行一个,如:\nhttps://tv.example.com/callback"} className="font-mono text-xs" {...field} />
                    </FormControl>
                    <FormDescription>{machine ? "机器客户端不使用浏览器回调，可留空" : "授权码模式将仅允许跳转至这些地址"}</FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="grantTypes"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      授权类型 <span className="text-destructive">*</span>
                    </FormLabel>
                    <div className="grid grid-cols-1 gap-2 rounded-lg border p-3 sm:grid-cols-2">
                      {GRANT_TYPES.filter(g => machine ? g === "client_credentials" : subjectMode === "LEGACY_MIXED" || g !== "client_credentials").map((g) => {
                        const checked = field.value.includes(g)
                        return (
                          <label key={g} className="flex cursor-pointer items-start gap-2 text-sm">
                            <Checkbox
                              checked={checked}
                              disabled={machine || writeBusy}
                              onCheckedChange={(v) =>
                                field.onChange(v ? [...field.value, g] : field.value.filter((x) => x !== g))
                              }
                              aria-label={GRANT_TYPE_LABELS[g]}
                            />
                            <span className="min-w-0">
                              <span className="block truncate">{GRANT_TYPE_LABELS[g]}</span>
                              <span className="block truncate font-mono text-[10px] text-muted-foreground">
                                {g === "urn:ietf:params:oauth:grant-type:device_code" ? "device_code" : g}
                              </span>
                            </span>
                          </label>
                        )
                      })}
                    </div>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="scopes"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>申请权限范围(scopes)</FormLabel>
                    <div className="flex flex-wrap gap-2">
                      {SCOPES.map((s) => {
                        const checked = field.value.includes(s)
                        return (
                          <label
                            key={s}
                            className={`flex cursor-pointer items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs transition-colors ${
                              checked ? "border-primary/40 bg-primary/10 text-primary" : "hover:bg-muted/60"
                            }`}
                          >
                            <Checkbox
                              checked={checked}
                              onCheckedChange={(v) =>
                                field.onChange(v ? [...field.value, s] : field.value.filter((x) => x !== s))
                              }
                              aria-label={s}
                            />
                            {s}
                          </label>
                        )
                      })}
                    </div>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="accessTokenValidity"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>访问令牌有效期(秒)</FormLabel>
                      <FormControl>
                        <Input type="number" min={60} max={86400} {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="refreshTokenValidity"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>刷新令牌有效期(秒)</FormLabel>
                      <FormControl>
                        <Input type="number" min={60} max={2592000} {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              <FormField
                control={form.control}
                name="description"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>描述</FormLabel>
                    <FormControl>
                      <Textarea rows={2} placeholder="选填" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </form>
          </Form>
          <DialogFooter className="border-t pt-4">
            <Button variant="outline" onClick={() => setDialogOpen(false)} disabled={saveMutation.isPending}>
              取消
            </Button>
            <Button type="submit" form="client-form" disabled={saveMutation.isPending}>
              {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
              {editing ? "保存更改" : "创建并生成密钥"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <SecretDialog open={!!secret} onOpenChange={(o) => !o && setSecret(null)} secret={secret ?? ""} />
      {policyTarget && <AdmissionPolicyDialog key={policyTarget.id} target={policyTarget} onClose={() => setPolicyTarget(null)}
        onSaved={fence => { useAuthStore.getState().retireAuthorizationProjection(fence); void queryClient.invalidateQueries({ queryKey: ["oauth-clients"] }) }} />}

      <ConfirmDialog
        open={!!deleting}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={`删除客户端「${deleting?.clientName ?? ""}」?`}
        description="将删除该客户端及其全部令牌与设备码记录,不可撤销。"
        loading={deleteMutation.isPending}
        onConfirm={() => { if (deleting && startWrite()) deleteMutation.mutate(deleting.id) }}
      />
    </div>
  )
}

function formatDuration(seconds?: number): string {
  if (!seconds) return "—"
  if (seconds < 3600) return `${formatNumber(seconds / 60 | 0)} 分钟`
  if (seconds < 86400) return `${formatNumber((seconds / 3600) | 0)} 小时`
  return `${formatNumber((seconds / 86400) | 0)} 天`
}
