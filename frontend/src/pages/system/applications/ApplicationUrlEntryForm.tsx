import * as React from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { Globe, Loader2, RotateCcw } from "lucide-react"
import { toast } from "sonner"
import { applicationsApi } from "@/api/applications"
import type { ApplicationEnvironment, ApplicationRecord, ApplicationUrlEntryRecord } from "@/api/types"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Form, FormControl, FormDescription, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
import { zResolver } from "@/lib/form"
import { urlEntrySchema, type UrlEntryValues } from "./url-entry-policy"
import { isQiankunApp } from "./application-setup-policy"

type Props = {
  application: ApplicationRecord
  environment: ApplicationEnvironment
  entry: ApplicationUrlEntryRecord | null
  ready: boolean
  busy: boolean
  onBusyChange: (busy: boolean) => void
  onChanged: () => void
}

/** Key this editor by application and environment so a pending write cannot cross targets. */
export function ApplicationUrlEntryForm({ application, environment, entry, ready, busy, onBusyChange, onChanged }: Props) {
  const queryClient = useQueryClient()
  const reconfigure = !isQiankunApp(application)
  const mounted = React.useRef(true)
  React.useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  const form = useForm<UrlEntryValues>({ resolver: zResolver(urlEntrySchema),
    defaultValues: { entryUrl: entry?.entryUrl ?? "", proxyEnabled: entry?.proxyEnabled ?? true } })
  React.useEffect(() => {
    if (ready) form.reset({ entryUrl: entry?.entryUrl ?? "", proxyEnabled: entry?.proxyEnabled ?? true })
  }, [ready, entry?.entryUrl, entry?.proxyEnabled, form])
  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["application-url-entry", application.id, environment] }),
      queryClient.invalidateQueries({ queryKey: ["applications"] }),
      queryClient.invalidateQueries({ queryKey: ["profile"] }),
    ])
    if (mounted.current) onChanged()
  }
  const save = useMutation({
    mutationFn: async (values: UrlEntryValues) => {
      if (reconfigure) {
        await applicationsApi.update(application.id, {
          code: application.code, name: application.name, icon: application.icon || "LayoutGrid",
          basePath: application.basePath, description: application.description ?? "", sortOrder: application.sortOrder ?? 0,
          status: "DISABLED", entryType: "HTML", runtimeMode: "TRUSTED", entryUrl: values.entryUrl,
        })
      }
      try { return await applicationsApi.setUrlEntry(application.id, environment, values) }
      catch (error) {
        if (reconfigure) {
          await queryClient.invalidateQueries({ queryKey: ["applications"] })
          throw new Error(`qiankun 基础配置已保存，应用保持停用；入口配置尚未完成，请重试。${error instanceof Error ? error.message : ""}`, { cause: error })
        }
        throw error
      }
    },
    onSuccess: async (next) => {
      queryClient.setQueryData(["application-url-entry", application.id, environment], next)
      if (mounted.current) toast.success(reconfigure ? "qiankun 入口已重新配置，应用保持停用" : "直接地址已保存", {
        description: reconfigure ? "确认子应用已适配生命周期后，请在应用信息中启用。原有菜单与授权继续保留。" : `${environment} 环境使用新入口，现有访问授权继续生效。`,
      })
      await refresh()
    },
    onError: (error: Error) => { if (mounted.current) toast.error(error.message || "入口保存失败") },
  })
  const remove = useMutation({
    mutationFn: () => applicationsApi.removeUrlEntry(application.id, environment),
    onSuccess: async () => {
      queryClient.setQueryData(["application-url-entry", application.id, environment], null)
      if (mounted.current) toast.success("已移除直接地址", { description: "当前环境恢复使用已激活版本；若没有版本，请先发布。" })
      await refresh()
    },
    onError: (error: Error) => { if (mounted.current) toast.error(error.message || "入口移除失败") },
  })
  const pending = save.isPending || remove.isPending
  React.useEffect(() => { onBusyChange(pending); return () => onBusyChange(false) }, [pending, onBusyChange])
  const disabled = busy || pending || !ready

  return <section className="mb-6 rounded-xl border border-primary/25 bg-primary/[0.025] p-4" aria-label="已部署应用接入">
    <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
      <div><h3 className="flex items-center gap-2 text-sm font-semibold"><Globe className="size-4 text-primary" />已部署应用</h3>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">填写应用地址，保存后即可使用，无需上传文件或填写版本号。</p></div>
      <Badge variant={entry && !reconfigure ? "default" : "outline"}>{reconfigure ? "需重新配置" : entry ? "当前使用" : "按环境配置"}</Badge>
    </div>
    <Form {...form}><form className="space-y-4" onSubmit={form.handleSubmit(values => { if (!disabled) save.mutate(values) })}>
      {reconfigure && <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3 text-xs leading-relaxed" role="note">
        此应用需要重新配置 qiankun 入口。保存时会改为 qiankun HTML 应用并保持停用；子应用代码将在平台页面中执行，请确认已导出标准生命周期并完成审核。配置成功后，可在应用信息中启用。
      </div>}
      <FormField control={form.control} name="entryUrl" render={({ field }) => <FormItem>
        <FormLabel>应用地址</FormLabel><FormControl><Input {...field} type="url" placeholder="https://app1.example.com/" className="font-mono" disabled={disabled} /></FormControl>
        <FormDescription>填写应用首页、目录地址或 HTML 页面地址。</FormDescription><FormMessage />
      </FormItem>} />
      <details className="rounded-lg border bg-background/70">
        <summary className="cursor-pointer px-3 py-2.5 text-sm font-medium">高级设置 · {form.watch("proxyEnabled") ? "已启用平台代理" : "直接访问应用"}</summary>
        <div className="space-y-3 border-t p-3">
      <FormField control={form.control} name="proxyEnabled" render={({ field }) => <FormItem className="flex items-center justify-between gap-4 rounded-lg border bg-background/70 p-3">
        <div><FormLabel>通过平台同域代理</FormLabel><FormDescription className="mt-1">页面和相对静态资源通过平台域名访问；业务 API 仍需单独配置。</FormDescription></div>
        <FormControl><Switch checked={field.value} onCheckedChange={field.onChange} disabled={disabled} aria-label="通过平台同域代理" /></FormControl>
      </FormItem>} />
      {entry && <p className="break-all text-xs text-muted-foreground">浏览器加载入口 <code className="font-mono">{entry.browserEntryUrl}</code></p>}
        </div>
      </details>
      <div className="flex flex-wrap items-center justify-between gap-3 border-t pt-3">
        <p className="text-xs text-muted-foreground">当前配置仅影响 {environment} 环境，菜单与访问范围继续由平台管理。</p>
        <div className="flex gap-2">{entry && <Button type="button" variant="outline" size="sm" disabled={disabled} onClick={() => { if (!disabled) remove.mutate() }}>{remove.isPending ? <Loader2 className="size-4 animate-spin" /> : <RotateCcw className="size-4" />}恢复版本入口</Button>}
          <Button type="submit" size="sm" disabled={disabled}>{save.isPending && <Loader2 className="size-4 animate-spin" />}{reconfigure ? "保存 qiankun 配置（保持停用）" : "保存并使用"}</Button></div>
      </div>
    </form></Form>
  </section>
}
