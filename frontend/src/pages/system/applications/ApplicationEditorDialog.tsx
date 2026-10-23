import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Form, FormControl, FormDescription, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Switch } from "@/components/ui/switch"
import { IconInput } from "@/components/IconInput"
import { ArrowRight, ChevronDown, Layers, Loader2 } from "lucide-react"
import { isMicroApp, type useApplicationEditor } from "./use-application-editor"

export function ApplicationEditorDialog({ editor }: { editor: ReturnType<typeof useApplicationEditor> }) {
  const {
    dialogOpen, setDialogOpen, editing, quickSetup, setQuickSetup, form,
    codeValue, useQuickSetup, saveMutation, submit,
  } = editor
  const generatedPath = `/micro/${codeValue || "{code}"}`
  const requiresConfiguration = !!editing && isMicroApp(editing)
    && (editing.entryType !== "HTML" || editing.runtimeMode !== "TRUSTED")
  const advancedError = (["basePath", "icon", "description", "sortOrder", "status"] as const)
    .some((key) => Boolean(form.formState.errors[key]))

  return (
    <Dialog open={dialogOpen} onOpenChange={(open) => !saveMutation.isPending && setDialogOpen(open)}>
      <DialogContent className="max-h-[90dvh] grid-rows-[auto_minmax(0,1fr)_auto] sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{editing ? "编辑应用" : "创建微应用"}</DialogTitle>
          <DialogDescription>
            {editing ? `编辑「${editing.name}」的基本信息` : "先填写名称和编码，创建后配置入口地址与访问范围。"}
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form id="app-form" onSubmit={submit} className="min-h-0 space-y-5 overflow-y-auto overscroll-contain pr-1">
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField control={form.control} name="name" render={({ field }) => (
                <FormItem>
                  <FormLabel>应用名称 <span className="text-destructive">*</span></FormLabel>
                  <FormControl><Input placeholder="如：客户管理" {...field} /></FormControl>
                  <FormMessage />
                </FormItem>
              )} />
              <FormField control={form.control} name="code" render={({ field }) => (
                <FormItem>
                  <FormLabel>应用编码 <span className="text-destructive">*</span></FormLabel>
                  <FormControl><Input placeholder="如：crm" className="font-mono" {...field} disabled={!!editing} /></FormControl>
                  <FormDescription>{editing ? "编码创建后不可修改" : "小写字母、数字与中划线"}</FormDescription>
                  <FormMessage />
                </FormItem>
              )} />
            </div>

            {!editing ? (
              <div className="flex gap-3 rounded-lg border border-primary/15 bg-primary/5 px-4 py-3">
                <Layers className="mt-0.5 size-4 shrink-0 text-primary" />
                <div className="min-w-0 space-y-1">
                  <p className="text-sm font-medium">qiankun 微应用 · 独立部署</p>
                  <p className="text-xs leading-relaxed text-muted-foreground">
                    {useQuickSetup ? "自动生成访问路径和首页菜单，下一步填写 HTTP/HTTPS 入口或上传构建包。" : "使用自定义路径，创建后继续配置菜单与应用入口。"}
                  </p>
                  <code className="block break-all font-mono text-xs text-primary">{useQuickSetup ? generatedPath : form.watch("basePath") || "尚未填写自定义路径"}</code>
                </div>
              </div>
            ) : (
              <div className="rounded-lg border bg-muted/30 px-4 py-3 text-xs leading-relaxed text-muted-foreground">
                {requiresConfiguration ? "此应用需先在应用入口中明确重新配置为 qiankun，再编辑或启用。" : isMicroApp(editing) ? "qiankun 应用入口按环境配置，子应用需导出标准生命周期。此处仅编辑基本信息。" : "宿主内置页面随宿主发布，此处编辑应用元数据。"}
              </div>
            )}

            <details key={editing?.id ?? "create"} open={advancedError || undefined} className="group rounded-lg border">
              <summary className="flex cursor-pointer list-none items-center justify-between gap-3 rounded-lg px-4 py-3 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                更多设置
                <ChevronDown className="size-4 text-muted-foreground transition-transform group-open:rotate-180" />
              </summary>
              <div className="space-y-4 border-t p-4">
                {!editing && (
                  <div className="flex items-center justify-between gap-4">
                    <div>
                      <p className="text-sm font-medium">自动配置首页</p>
                      <p className="text-xs text-muted-foreground">生成标准访问路径和首页菜单</p>
                    </div>
                    <Switch checked={quickSetup} onCheckedChange={setQuickSetup} aria-label="自动配置首页" />
                  </div>
                )}
                <FormField control={form.control} name="basePath" render={({ field }) => (
                  <FormItem>
                    <FormLabel>访问路径</FormLabel>
                    <FormControl><Input className="font-mono" placeholder="如：/micro/crm" {...field} value={!editing && useQuickSetup ? generatedPath : field.value} readOnly={!editing && useQuickSetup} /></FormControl>
                    <FormMessage />
                  </FormItem>
                )} />
                <FormField control={form.control} name="icon" render={({ field }) => (
                  <FormItem>
                    <FormLabel>图标</FormLabel>
                    <FormControl><IconInput value={field.value ?? ""} onChange={field.onChange} /></FormControl>
                    <FormMessage />
                  </FormItem>
                )} />
                <FormField control={form.control} name="description" render={({ field }) => (
                  <FormItem>
                    <FormLabel>描述</FormLabel>
                    <FormControl><Textarea rows={2} placeholder="选填" {...field} /></FormControl>
                    <FormMessage />
                  </FormItem>
                )} />
                <div className="grid gap-4 sm:grid-cols-2">
                  <FormField control={form.control} name="sortOrder" render={({ field }) => (
                    <FormItem>
                      <FormLabel>排序值</FormLabel>
                      <FormControl><Input type="number" min={0} {...field} /></FormControl>
                      <FormMessage />
                    </FormItem>
                  )} />
                  <FormField control={form.control} name="status" render={({ field }) => (
                    <FormItem>
                      <FormLabel>状态</FormLabel>
                      <Select onValueChange={field.onChange} value={field.value}>
                        <FormControl><SelectTrigger className="w-full"><SelectValue /></SelectTrigger></FormControl>
                        <SelectContent><SelectItem value="ENABLED">启用</SelectItem><SelectItem value="DISABLED">停用</SelectItem></SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )} />
                </div>
              </div>
            </details>
          </form>
        </Form>
        <DialogFooter>
          <Button variant="outline" onClick={() => setDialogOpen(false)} disabled={saveMutation.isPending}>取消</Button>
          <Button type="submit" form="app-form" disabled={saveMutation.isPending || requiresConfiguration}>
            {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
            {editing ? "保存更改" : <>创建并继续 <ArrowRight className="size-4" /></>}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
