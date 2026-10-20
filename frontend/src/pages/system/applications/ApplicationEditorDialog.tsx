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
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Switch } from "@/components/ui/switch"
import { IconInput } from "@/components/IconInput"
import { Loader2 } from "lucide-react"
import { isMicroApp, isHostEsm, type useApplicationEditor } from "./use-application-editor"

export function ApplicationEditorDialog({ editor }: { editor: ReturnType<typeof useApplicationEditor> }) {
  const { dialogOpen, setDialogOpen, editing, quickSetup, setQuickSetup, form,
    entryTypeValue, codeValue, useQuickSetup, saveMutation, submit } = editor
  return (
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{editing ? "编辑应用" : "新建微应用"}</DialogTitle>
            <DialogDescription>
              {editing ? `正在编辑「${editing.name}」` : "应用编码创建后不可修改"}
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form id="app-form" onSubmit={submit} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        应用名称 <span className="text-destructive">*</span>
                      </FormLabel>
                      <FormControl>
                        <Input placeholder="如: 数据分析" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="code"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        应用编码 <span className="text-destructive">*</span>
                      </FormLabel>
                      <FormControl>
                        <Input placeholder="如: athena" className="font-mono" {...field} disabled={!!editing} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="icon"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>图标</FormLabel>
                      <FormControl>
                        <IconInput value={field.value ?? ""} onChange={field.onChange} />
                      </FormControl>
                      <FormDescription>从图标库中选择,展示于应用切换器与工作台</FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="basePath"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>基础路径</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="如 /athena"
                          className="font-mono"
                          {...field}
                          value={!editing && (useQuickSetup || entryTypeValue === "ESM") ? `/micro/${codeValue || "{code}"}` : field.value}
                          readOnly={!editing && (useQuickSetup || entryTypeValue === "ESM")}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              {editing && isHostEsm(editing) ? (
                <div className="rounded-md border bg-muted/30 px-3 py-3 text-sm">
                  <p className="font-medium">第一方 ESM · 宿主直挂</p>
                  <p className="mt-1 text-xs text-muted-foreground">模块入口随宿主构建部署，不需要单独上传版本。</p>
                </div>
              ) : <>
              {editing ? (
                <div className="rounded-md border bg-muted/30 px-3 py-2 text-sm">接入类型：{!isMicroApp(editing) ? "宿主内置路由" : entryTypeValue === "ESM" ? "ESM · 随宿主构建" : "HTML · 上传版本包"}</div>
              ) : <FormField control={form.control} name="entryType" render={({ field }) => (
                <FormItem><FormLabel>接入类型</FormLabel>
                  <Select value={field.value} onValueChange={field.onChange}>
                    <FormControl><SelectTrigger className="w-full"><SelectValue /></SelectTrigger></FormControl>
                    <SelectContent><SelectItem value="HTML">HTML · 上传版本包 / iframe</SelectItem><SelectItem value="ESM">ESM · 宿主构建 / 直挂</SelectItem></SelectContent>
                  </Select>
                  <FormDescription>ESM 仅适用于已纳入宿主构建并由平台管理员审核的第一方模块。</FormDescription>
                  <FormMessage />
                </FormItem>
              )} />}
              {entryTypeValue === "HTML" && !editing && (
                <div className="flex items-center justify-between gap-4 rounded-md border bg-muted/30 px-3 py-2.5">
                  <div>
                    <p className="text-sm font-medium">快速接入</p>
                    <p className="text-xs text-muted-foreground">按编码生成挂载路径和首页菜单，上传版本后才可访问</p>
                  </div>
                  <Switch checked={quickSetup} onCheckedChange={setQuickSetup} aria-label="快速接入微应用" />
                </div>
              )}
              {!editing && entryTypeValue === "ESM" ? <p className="text-xs leading-relaxed text-muted-foreground">应用编码自动对应宿主内同名 ESM 模块。先将模块加入宿主构建；创建时无需填写入口。</p> : !editing && entryTypeValue === "HTML" ? <p className="text-xs leading-relaxed text-muted-foreground">HTML 应用使用 iframe 隔离。创建后上传 ZIP 或 tar.gz，系统生成固定版本入口；未激活前不会加载占位入口。</p> : null}
              </>}
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
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="sortOrder"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>排序值</FormLabel>
                      <FormControl>
                        <Input type="number" min={0} {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="status"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>状态</FormLabel>
                      <Select onValueChange={field.onChange} value={field.value}>
                        <FormControl>
                          <SelectTrigger className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          <SelectItem value="ENABLED">启用</SelectItem>
                          <SelectItem value="DISABLED">停用</SelectItem>
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
              <p className="text-xs leading-relaxed text-muted-foreground">{editing && !isMicroApp(editing)
                ? "宿主内置路由随宿主前端发布；此处只编辑应用元数据。"
                : editing && isHostEsm(editing)
                ? "该第一方模块随宿主前端发布；租户、组织和角色授权仍由平台配置。"
                : entryTypeValue === "ESM"
                  ? "ESM 模块需先加入宿主构建，创建后配置菜单与授权；代码更新随宿主部署。"
                  : "HTML 微应用创建后上传 ZIP 或 tar.gz，预检、激活并配置授权；版本文件独立于宿主镜像。"}</p>
            </form>
          </Form>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)} disabled={saveMutation.isPending}>
              取消
            </Button>
            <Button type="submit" form="app-form" disabled={saveMutation.isPending}>
              {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
              {editing ? "保存更改" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

  )
}
