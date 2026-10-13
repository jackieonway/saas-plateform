import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { Building2, Image, Loader2, Save, Settings2, ShieldCheck } from "lucide-react"
import { z } from "zod"
import { toast } from "sonner"
import { platformApi, type PlatformSettings, type PlatformSettingsPayload } from "@/api/platform"
import { BrandMark } from "@/components/brand"
import { PageHeader } from "@/components/layout/PageHeader"
import { EmptyState, ErrorState } from "@/components/states"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Form, FormControl, FormDescription, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { DEFAULT_PLATFORM_BRANDING, PLATFORM_BRANDING_QUERY_KEY } from "@/hooks/use-platform-branding"
import { zResolver } from "@/lib/form"
import { useAuthStore } from "@/stores/auth"

function validLogoUrl(value: string): boolean {
  if (!value) return true
  let decoded = value
  for (let count = 0; count < 8; count += 1) {
    if (/[\u0000-\u001f\u007f-\u009f\\]/.test(decoded)) return false
    try {
      if (decoded.startsWith("/")) {
        if (decoded.startsWith("//") || decoded.split(/[?#]/)[0].split("/").some((part) => part === "." || part === "..")) return false
        if (new URL(`https://platform.invalid${decoded}`).hostname !== "platform.invalid") return false
      } else {
        if (!/^https?:\/\//i.test(decoded)) return false
        const url = new URL(decoded)
        if (!url.hostname || url.username || url.password) return false
      }
      const next = decodeURIComponent(decoded.replace(/%(?![0-9a-f]{2})/gi, "%25"))
      if (next === decoded) return true
      decoded = next
    } catch {
      return false
    }
  }
  return false
}

function textField(label: string, maximum: number, required = false) {
  return z.string()
    .refine((value) => !/[\u0000-\u001f\u007f-\u009f]/.test(value), `${label}不能包含控制字符`)
    .trim()
    .refine((value) => !required || value.length > 0, `请输入${label}`)
    .refine((value) => Array.from(value).length <= maximum, `${label}最多 ${maximum} 个字符`)
}

const settingsSchema = z.object({
  platformName: textField("平台名称", 100, true),
  platformDescription: textField("平台描述", 300),
  logoUrl: textField("Logo 地址", 2048).refine(validLogoUrl, "请填写安全的 HTTP(S) 地址或以 / 开头的站内路径"),
  loginTitle: textField("登录页标题", 100, true),
  loginDescription: textField("登录页说明", 300),
  footerText: textField("页脚文案", 500),
  platformTenantName: textField("平台租户名称", 100, true),
})

function writableSettings(settings: PlatformSettings): PlatformSettingsPayload {
  return {
    platformName: settings.platformName,
    platformDescription: settings.platformDescription,
    logoUrl: settings.logoUrl,
    loginTitle: settings.loginTitle,
    loginDescription: settings.loginDescription,
    footerText: settings.footerText,
    platformTenantName: settings.platformTenantName,
  }
}

export default function PlatformSettingsPage() {
  const queryClient = useQueryClient()
  const token = useAuthStore((s) => s.token)
  const epoch = useAuthStore((s) => s.contextEpoch)
  const profile = useAuthStore((s) => s.profile)
  const permissionsQuery = usePermissionsQuery()
  const isPlatformTenant = profile?.tenant.isPlatform ?? false
  const isSuper = profile?.user.isSuperAdmin ?? false
  const canManage = isPlatformTenant && (isSuper || permissionsQuery.data?.permissions.includes("system:settings") === true)

  const settingsQuery = useQuery({
    queryKey: ["platform-settings", epoch],
    queryFn: ({ signal }) => platformApi.settings(token!, signal),
    enabled: !!token && canManage,
    retry: 1,
    staleTime: 30_000,
  })
  const form = useForm<PlatformSettingsPayload>({
    resolver: zResolver(settingsSchema),
    defaultValues: { ...DEFAULT_PLATFORM_BRANDING, platformTenantName: "" },
  })
  const isDirty = form.formState.isDirty
  React.useEffect(() => {
    if (settingsQuery.data && !isDirty) form.reset(writableSettings(settingsQuery.data))
  }, [settingsQuery.data, isDirty, form])

  const saveMutation = useMutation({
    mutationFn: ({ values, sessionToken }: { values: PlatformSettingsPayload; sessionToken: string; contextEpoch: number }) =>
      platformApi.updateSettings(values, sessionToken),
    onSuccess: async (settings, variables) => {
      const { platformTenantName: _name, platformTenantCode: _code, ...branding } = settings
      queryClient.setQueryData(PLATFORM_BRANDING_QUERY_KEY, branding)
      const current = useAuthStore.getState()
      if (current.token === variables.sessionToken && current.contextEpoch === variables.contextEpoch) {
        queryClient.setQueryData(["platform-settings", variables.contextEpoch], settings)
        form.reset(writableSettings(settings))
      }
      toast.success("平台设置已保存")
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: PLATFORM_BRANDING_QUERY_KEY }),
        queryClient.invalidateQueries({ queryKey: ["platform-settings"] }),
        queryClient.invalidateQueries({ queryKey: ["tenants"] }),
        queryClient.invalidateQueries({ queryKey: ["profile"] }),
        queryClient.invalidateQueries({ queryKey: ["auth-menus"] }),
        queryClient.invalidateQueries({ queryKey: ["permissions"] }),
      ])
    },
    onError: (error: Error) => toast.error(error.message || "平台设置保存失败"),
  })

  const values = form.watch()
  const [failedPreviewUrl, setFailedPreviewUrl] = React.useState<string | null>(null)
  const previewLogo = validLogoUrl(values.logoUrl) && values.logoUrl && failedPreviewUrl !== values.logoUrl

  if (!profile || (isPlatformTenant && !isSuper && permissionsQuery.isPending)) {
    return <div className="flex min-h-[40vh] items-center justify-center" role="status" aria-label="加载平台设置"><Loader2 className="size-6 animate-spin text-muted-foreground" /></div>
  }
  if (isPlatformTenant && !isSuper && permissionsQuery.isError) {
    return <ErrorState message="无法获取平台设置权限，请重试" onRetry={() => permissionsQuery.refetch()} />
  }
  if (!canManage) {
    return <EmptyState title="暂无平台设置权限" description="平台租户中拥有平台设置权限的管理员可修改此配置。" icon={<ShieldCheck className="size-6" />} />
  }
  if (settingsQuery.isPending) {
    return <div className="flex min-h-[40vh] items-center justify-center" role="status" aria-label="加载平台设置"><Loader2 className="size-6 animate-spin text-muted-foreground" /></div>
  }
  if (settingsQuery.isError || !settingsQuery.data) {
    return <ErrorState message={settingsQuery.error?.message} onRetry={() => settingsQuery.refetch()} />
  }

  return (
    <div className="mx-auto max-w-6xl">
      <PageHeader
        title="平台设置"
        description="统一管理平台品牌、登录页文案与平台租户名称"
        icon={<Settings2 className="size-5" />}
        badge={<Badge variant="secondary">全平台生效</Badge>}
      />
      <Form {...form}>
        <form onSubmit={form.handleSubmit((submitted) => {
          if (token) saveMutation.mutate({ values: submitted, sessionToken: token, contextEpoch: epoch })
        })}>
          <fieldset disabled={saveMutation.isPending} className="grid min-w-0 items-start gap-5 lg:grid-cols-[minmax(0,1fr)_320px]">
            <div className="space-y-5">
              <section className="rounded-xl border bg-card p-5">
                <div className="mb-5 flex items-center gap-2 text-sm font-semibold"><Building2 className="size-4 text-primary" />平台租户</div>
                <div className="grid gap-5 sm:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="platform-tenant-code">平台租户编码</Label>
                    <Input id="platform-tenant-code" value="admin-tenant" readOnly className="bg-muted/50 font-mono text-muted-foreground" />
                    <p className="text-xs text-muted-foreground">固定编码，用于识别平台管理租户。</p>
                  </div>
                  <FormField control={form.control} name="platformTenantName" render={({ field }) => (
                    <FormItem><FormLabel>平台租户名称</FormLabel><FormControl><Input {...field} placeholder="如：平台管理租户" /></FormControl><FormDescription>同步展示在租户管理与组织切换中。</FormDescription><FormMessage /></FormItem>
                  )} />
                </div>
              </section>

              <section className="rounded-xl border bg-card p-5">
                <div className="mb-5 flex items-center gap-2 text-sm font-semibold"><Image className="size-4 text-primary" />平台品牌</div>
                <div className="space-y-5">
                  <FormField control={form.control} name="platformName" render={({ field }) => (
                    <FormItem><FormLabel>平台名称</FormLabel><FormControl><Input {...field} placeholder="请输入平台名称" /></FormControl><FormDescription>用于导航、页面标题与平台通知。</FormDescription><FormMessage /></FormItem>
                  )} />
                  <FormField control={form.control} name="platformDescription" render={({ field }) => (
                    <FormItem><FormLabel>平台描述</FormLabel><FormControl><Textarea {...field} rows={2} placeholder="简要介绍平台" /></FormControl><FormDescription>用于品牌说明与浏览器页面描述。</FormDescription><FormMessage /></FormItem>
                  )} />
                  <FormField control={form.control} name="logoUrl" render={({ field }) => (
                    <FormItem><FormLabel>Logo 地址</FormLabel><FormControl><Input {...field} maxLength={2048} placeholder="https://example.com/logo.svg 或 /logo.svg" /></FormControl><FormDescription>支持 HTTP(S) 图片地址或站内路径；留空使用默认图标。</FormDescription><FormMessage /></FormItem>
                  )} />
                </div>
              </section>

              <section className="rounded-xl border bg-card p-5">
                <div className="mb-5 text-sm font-semibold">登录页与页脚</div>
                <div className="space-y-5">
                  <FormField control={form.control} name="loginTitle" render={({ field }) => (
                    <FormItem><FormLabel>登录页标题</FormLabel><FormControl><Input {...field} placeholder="如：欢迎回来" /></FormControl><FormMessage /></FormItem>
                  )} />
                  <FormField control={form.control} name="loginDescription" render={({ field }) => (
                    <FormItem><FormLabel>登录页说明</FormLabel><FormControl><Textarea {...field} rows={2} placeholder="请输入登录页说明" /></FormControl><FormMessage /></FormItem>
                  )} />
                  <FormField control={form.control} name="footerText" render={({ field }) => (
                    <FormItem><FormLabel>页脚文案</FormLabel><FormControl><Input {...field} placeholder="如：© 2026 示例平台" /></FormControl><FormDescription>显示于登录、授权与设备确认页；留空隐藏。</FormDescription><FormMessage /></FormItem>
                  )} />
                </div>
              </section>
            </div>

            <aside className="space-y-4 lg:sticky lg:top-0">
              <div className="overflow-hidden rounded-xl border bg-card">
                <div className="border-b bg-muted/30 px-5 py-3 text-sm font-medium">品牌预览</div>
                <div className="space-y-7 p-5">
                  <div className="flex items-center gap-3">
                    <span className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
                      {previewLogo ? <img src={values.logoUrl} alt="Logo 预览" className="size-8 object-contain" onError={() => setFailedPreviewUrl(values.logoUrl)} /> : <BrandMark className="size-8" logoUrl="" />}
                    </span>
                    <div className="min-w-0"><p className="break-words text-sm font-semibold">{values.platformName || "平台名称"}</p><p className="mt-1 break-words text-xs text-muted-foreground">{values.platformDescription}</p></div>
                  </div>
                  <div className="rounded-lg border bg-background p-4">
                    <p className="break-words text-lg font-semibold">{values.loginTitle || "登录页标题"}</p>
                    <p className="mt-2 break-words text-xs leading-relaxed text-muted-foreground">{values.loginDescription}</p>
                    <div className="mt-5 space-y-2" aria-hidden><div className="h-8 rounded-md bg-muted/60" /><div className="h-8 rounded-md bg-muted/60" /><div className="h-8 rounded-md bg-primary/80" /></div>
                  </div>
                  {values.footerText && <p className="break-words text-center text-[11px] leading-relaxed text-muted-foreground">{values.footerText}</p>}
                </div>
              </div>
              <p className="px-1 text-xs leading-relaxed text-muted-foreground">保存后当前页面立即更新，其他已打开的页面会自动同步。</p>
            </aside>
          </fieldset>

          {saveMutation.isError && <p role="alert" className="mt-5 rounded-lg border border-destructive/25 bg-destructive/5 p-3 text-sm text-destructive">{saveMutation.error.message}</p>}
          <div className="mt-5 flex items-center justify-end gap-3 border-t pt-5">
            {isDirty && <span className="mr-auto text-xs text-muted-foreground">有未保存的修改</span>}
            <Button type="button" variant="outline" disabled={!isDirty || saveMutation.isPending} onClick={() => { form.reset(writableSettings(settingsQuery.data!)); saveMutation.reset() }}>取消修改</Button>
            <Button type="submit" disabled={!isDirty || saveMutation.isPending}>
              {saveMutation.isPending ? <Loader2 className="size-4 animate-spin" /> : <Save className="size-4" />}
              {saveMutation.isPending ? "保存中..." : "保存设置"}
            </Button>
          </div>
        </form>
      </Form>
    </div>
  )
}
