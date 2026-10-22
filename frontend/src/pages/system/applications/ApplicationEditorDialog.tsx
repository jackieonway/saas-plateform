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
import { ArrowRight, ChevronDown, Code2, Layers, Loader2 } from "lucide-react"
import { isMicroApp, isHostEsm, type useApplicationEditor } from "./use-application-editor"

export function ApplicationEditorDialog({ editor }: { editor: ReturnType<typeof useApplicationEditor> }) {
  const {
    dialogOpen, setDialogOpen, editing, quickSetup, setQuickSetup, form,
    entryTypeValue, codeValue, useQuickSetup, hostEsmModules, selectHostEsmModule, saveMutation, submit,
  } = editor
  const esm = entryTypeValue === "ESM"
  const generatedPath = `/micro/${codeValue || "{code}"}`
  const advancedError = (["basePath", "icon", "description", "sortOrder", "status", "entryType"] as const)
    .some((key) => Boolean(form.formState.errors[key]))

  return (
    <Dialog open={dialogOpen} onOpenChange={(open) => !saveMutation.isPending && setDialogOpen(open)}>
      <DialogContent className="max-h-[90dvh] grid-rows-[auto_minmax(0,1fr)_auto] sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{editing ? "编辑应用" : "创建微应用"}</DialogTitle>
          <DialogDescription>
            {editing ? `编辑「${editing.name}」的基本信息` : "先填写名称和编码，创建后继续发布与访问配置。"}
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form id="app-form" onSubmit={submit} className="min-h-0 space-y-5 overflow-y-auto overscroll-contain pr-1">
            {!editing && esm && (
              <div className="space-y-2 rounded-lg border bg-muted/30 p-4">
                <label id="host-module-label" className="text-sm font-medium">选择第一方模块</label>
                <Select value={hostEsmModules.some((item) => item.code === codeValue) ? codeValue : ""} onValueChange={selectHostEsmModule} disabled={hostEsmModules.length === 0}>
                  <SelectTrigger className="w-full" aria-labelledby="host-module-label"><SelectValue placeholder="选择宿主中已构建的模块" /></SelectTrigger>
                  <SelectContent>{hostEsmModules.map((item) => <SelectItem key={item.code} value={item.code}>{item.code}</SelectItem>)}</SelectContent>
                </Select>
                <p className="text-xs leading-relaxed text-muted-foreground">
                  {hostEsmModules.length ? "编码与入口由模块自动确定。所选模块与宿主共享运行环境，请确认已完成审核。" : "当前宿主没有可选模块，请先将审核后的模块加入宿主构建。"}
                </p>
              </div>
            )}

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
                  <FormControl><Input placeholder="如：crm" className="font-mono" {...field} disabled={!!editing} readOnly={!editing && esm} /></FormControl>
                  <FormDescription>{editing ? "编码创建后不可修改" : esm ? "跟随所选模块" : "小写字母、数字与中划线"}</FormDescription>
                  <FormMessage />
                </FormItem>
              )} />
            </div>

            {!editing ? (
              <div className="flex gap-3 rounded-lg border border-primary/15 bg-primary/5 px-4 py-3">
                {esm ? <Code2 className="mt-0.5 size-4 shrink-0 text-primary" /> : <Layers className="mt-0.5 size-4 shrink-0 text-primary" />}
                <div className="min-w-0 space-y-1">
                  <p className="text-sm font-medium">{esm ? "第一方模块 · 随宿主发布" : "独立前端 · iframe 隔离"}</p>
                  <p className="text-xs leading-relaxed text-muted-foreground">
                    {esm ? "模块入口自动匹配，代码更新随宿主部署。" : useQuickSetup ? "自动生成访问路径和首页菜单，下一步上传构建包或填写已部署地址。" : "使用自定义路径，创建后继续配置菜单与发布版本。"}
                  </p>
                  <code className="block break-all font-mono text-xs text-primary">{useQuickSetup || esm ? generatedPath : form.watch("basePath") || "尚未填写自定义路径"}</code>
                </div>
              </div>
            ) : (
              <div className="rounded-lg border bg-muted/30 px-4 py-3 text-xs leading-relaxed text-muted-foreground">
                {isHostEsm(editing) ? "第一方模块随宿主构建发布，入口和运行方式保持现有配置。" : isMicroApp(editing) ? "独立前端的版本在接入面板中发布，此处编辑基本信息。" : "宿主内置页面随宿主发布，此处编辑应用元数据。"}
              </div>
            )}

            <details key={editing?.id ?? "create"} open={advancedError || undefined} className="group rounded-lg border">
              <summary className="flex cursor-pointer list-none items-center justify-between gap-3 rounded-lg px-4 py-3 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                更多设置
                <ChevronDown className="size-4 text-muted-foreground transition-transform group-open:rotate-180" />
              </summary>
              <div className="space-y-4 border-t p-4">
                {!editing && (
                  <FormField control={form.control} name="entryType" render={({ field }) => (
                    <FormItem>
                      <FormLabel>接入方式</FormLabel>
                      <Select value={field.value} onValueChange={field.onChange}>
                        <FormControl><SelectTrigger className="w-full"><SelectValue /></SelectTrigger></FormControl>
                        <SelectContent>
                          <SelectItem value="HTML">独立前端（推荐）</SelectItem>
                          <SelectItem value="ESM">第一方宿主模块</SelectItem>
                        </SelectContent>
                      </Select>
                      <FormDescription>React、Vue 或其他前端构建包使用独立前端接入。</FormDescription>
                      <FormMessage />
                    </FormItem>
                  )} />
                )}
                {!editing && !esm && (
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
                    <FormControl><Input className="font-mono" placeholder="如：/micro/crm" {...field} value={!editing && (useQuickSetup || esm) ? generatedPath : field.value} readOnly={!editing && (useQuickSetup || esm)} /></FormControl>
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
          <Button type="submit" form="app-form" disabled={saveMutation.isPending}>
            {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
            {editing ? "保存更改" : <>创建并继续 <ArrowRight className="size-4" /></>}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
