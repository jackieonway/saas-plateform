import { useFieldArray, useForm } from "react-hook-form"
import { Loader2, Plus, Trash2 } from "lucide-react"
import type { ApplicationEnvironment, ApplicationReleasePayload } from "@/api/types"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Form, FormControl, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
import { zResolver } from "@/lib/form"
import { externalReleasePayload, externalReleaseSchema, type ExternalReleaseValues } from "./external-release-policy"

export function ExternalReleaseForm({ environment, busy, pending, onSubmit }: {
  environment: ApplicationEnvironment
  busy: boolean
  pending: boolean
  onSubmit: (payload: ApplicationReleasePayload) => void
}) {
  const form = useForm<ExternalReleaseValues>({ resolver: zResolver(externalReleaseSchema),
    defaultValues: { version: "", entryUrl: "", healthUrl: "", resources: [] } })
  const resources = useFieldArray({ control: form.control, name: "resources" })
  return (
    <section className="mb-5 rounded-lg border p-4" aria-label="登记已部署 URL">
      <div className="mb-4">
        <h3 className="text-sm font-semibold">登记 {environment} 外部入口</h3>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">先部署固定版本的 HTML，再登记公开地址。入口须含版本目录；登记后为待预检状态，通过预检后手动激活。</p>
      </div>
      <Form {...form}>
        <form className="space-y-4" onSubmit={form.handleSubmit(values => { if (!busy) onSubmit(externalReleasePayload(environment, values)) })}>
          <FormField control={form.control} name="version" render={({ field }) => (
            <FormItem><FormLabel>版本号 <span className="text-destructive">*</span></FormLabel><FormControl><Input {...field} placeholder="如 1.0.0" className="font-mono" disabled={busy} /></FormControl><FormMessage /></FormItem>
          )} />
          <FormField control={form.control} name="entryUrl" render={({ field }) => (
            <FormItem><FormLabel>HTML 入口 URL <span className="text-destructive">*</span></FormLabel><FormControl><Input {...field} placeholder="/sub-apps/app/versions/1.0.0/index.html" className="font-mono" disabled={busy} /></FormControl><p className="text-xs text-muted-foreground">网关根路径或批准 origin 的绝对 HTTPS URL；服务端仍检查白名单。入口类型固定 HTML。</p><FormMessage /></FormItem>
          )} />
          <FormField control={form.control} name="healthUrl" render={({ field }) => (
            <FormItem><FormLabel>健康 URL（可选）</FormLabel><FormControl><Input {...field} placeholder="https://approved-origin/healthz" className="font-mono" disabled={busy} /></FormControl><p className="text-xs text-muted-foreground">使用网关根路径或绝对 HTTPS URL；根路径相对平台网关解析。检查此公开地址不等于读取 Docker 容器健康状态。</p><FormMessage /></FormItem>
          )} />
          <fieldset className="space-y-3">
            <legend className="text-sm font-medium">显式资源（可选，最多 48 项）</legend>
            <p className="text-xs leading-relaxed text-muted-foreground">预检自动发现入口资源；可补充动态 chunk。相对路径按 HTML 入口解析。</p>
            {resources.fields.map((resource, index) => (
              <div key={resource.id} className="space-y-2 rounded-md border bg-muted/20 p-3" role="group" aria-label={`资源 ${index + 1}`}>
                <div className="flex items-end gap-2">
                  <FormField control={form.control} name={`resources.${index}.kind`} render={({ field }) => (
                    <FormItem className="w-32 shrink-0"><FormLabel>资源 {index + 1} 类型</FormLabel><Select value={field.value} onValueChange={field.onChange} disabled={busy}><FormControl><SelectTrigger><SelectValue /></SelectTrigger></FormControl><SelectContent><SelectItem value="SCRIPT">脚本</SelectItem><SelectItem value="STYLE">样式</SelectItem><SelectItem value="ASSET">资源</SelectItem></SelectContent></Select><FormMessage /></FormItem>
                  )} />
                  <Button type="button" variant="ghost" size="icon" disabled={busy} aria-label={`移除资源 ${index + 1}`} onClick={() => resources.remove(index)}><Trash2 className="size-4" /></Button>
                </div>
                <FormField control={form.control} name={`resources.${index}.url`} render={({ field }) => (
                  <FormItem><FormLabel>资源 {index + 1} URL</FormLabel><FormControl><Input {...field} placeholder="assets/business-chunk.js" className="font-mono" disabled={busy} /></FormControl><FormMessage /></FormItem>
                )} />
              </div>
            ))}
            <Button type="button" variant="outline" size="sm" disabled={busy || resources.fields.length >= 48} onClick={() => resources.append({ kind: "SCRIPT", url: "" })}><Plus className="size-4" />添加资源</Button>
            {form.formState.errors.resources?.root?.message && <p className="text-sm text-destructive" role="alert">{form.formState.errors.resources.root.message}</p>}
          </fieldset>
          <div className="flex items-center justify-between gap-3 border-t pt-3">
            <p className="text-xs text-muted-foreground">仅登记，不自动预检、激活或新增授权。</p>
            <Button type="submit" size="sm" disabled={busy}>{pending && <Loader2 className="size-4 animate-spin" />}登记 URL</Button>
          </div>
        </form>
      </Form>
    </section>
  )
}
