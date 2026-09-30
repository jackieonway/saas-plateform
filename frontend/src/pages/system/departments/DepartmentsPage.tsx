import * as React from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import { Building2, Loader2, Plus } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form"
import { PageHeader } from "@/components/layout/PageHeader"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { TreeEditor, type EditableNode } from "@/components/TreeEditor"
import { EntityTreePicker, type PickerNode } from "@/components/EntityTreePicker"
import { EmptyState, ErrorState } from "@/components/states"
import { tenantsApi } from "@/api/tenants"
import { organizationsApi } from "@/api/organizations"
import { departmentsApi } from "@/api/departments"
import { asArray, asPage } from "@/lib/api"
import { toast } from "sonner"
import type { DeptNode, OrgNode, TenantRecord } from "@/api/types"

const deptSchema = z.object({
  name: z.string().min(1, "请输入部门名称").max(64),
  code: z.string().min(1, "请输入编码").max(32).regex(/^[a-zA-Z0-9-]+$/, "仅支持字母、数字与中划线"),
  sortOrder: z.coerce.number().int().min(0).max(9999),
  description: z.string().max(255).optional(),
})
type DeptForm = z.infer<typeof deptSchema>

function deptToEditable(nodes: DeptNode[]): EditableNode[] {
  return nodes.map((n) => ({
    id: n.id,
    name: n.name,
    code: n.code,
    description: n.description,
    children: n.children ? deptToEditable(n.children) : [],
  }))
}

function findDept(nodes: DeptNode[], id: string): DeptNode | null {
  for (const n of nodes) {
    if (n.id === id) return n
    const sub = n.children?.length ? findDept(n.children, id) : null
    if (sub) return sub
  }
  return null
}

export default function DepartmentsPage() {
  const queryClient = useQueryClient()

  const tenantsQuery = useQuery({
    queryKey: ["tenants", "options"],
    queryFn: async () => asPage<TenantRecord>(await tenantsApi.list({ page: 1, size: 100 })).records,
    staleTime: 60_000,
    retry: 1,
  })
  const tenants = tenantsQuery.data ?? []
  const [tenantId, setTenantId] = React.useState<string | null>(null)
  React.useEffect(() => {
    if (!tenantId && tenants.length > 0) setTenantId(tenants[0].id)
  }, [tenants, tenantId])

  const orgTreeQuery = useQuery({
    queryKey: ["org-tree", tenantId],
    queryFn: async () => asArray<OrgNode>(await organizationsApi.tree(tenantId!)),
    enabled: !!tenantId,
    staleTime: 30_000,
    retry: 1,
  })
  const orgTree = orgTreeQuery.data ?? []
  const [organizationId, setOrganizationId] = React.useState<string | null>(null)
  React.useEffect(() => {
    if (organizationId && orgTree.length > 0) {
      const exists = (function find(nodes: OrgNode[], id: string): boolean {
        return nodes.some((n) => n.id === id || (n.children?.length ? find(n.children, id) : false))
      })(orgTree, organizationId)
      if (!exists) setOrganizationId(null)
    }
  }, [orgTree, organizationId])

  const selectedOrg = React.useMemo(() => {
    const find = (nodes: OrgNode[]): OrgNode | null => {
      for (const n of nodes) {
        if (n.id === organizationId) return n
        const sub = n.children?.length ? find(n.children) : null
        if (sub) return sub
      }
      return null
    }
    return organizationId ? find(orgTree) : null
  }, [orgTree, organizationId])

  const treeQuery = useQuery({
    queryKey: ["dept-tree", organizationId],
    queryFn: async () => asArray<DeptNode>(await departmentsApi.tree(organizationId!)),
    enabled: !!organizationId,
    staleTime: 30_000,
    retry: 1,
  })
  const deptTree = treeQuery.data ?? []

  /* ---- CRUD ---- */
  const [dialogOpen, setDialogOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<DeptNode | null>(null)
  const [parent, setParent] = React.useState<string | null>(null)
  const form = useForm<DeptForm>({
    resolver: zResolver(deptSchema),
    defaultValues: { name: "", code: "", sortOrder: 0, description: "" },
  })

  const openCreate = (parentId: string | null) => {
    setEditing(null)
    setParent(parentId)
    form.reset({ name: "", code: "", sortOrder: 0, description: "" })
    setDialogOpen(true)
  }
  const openEdit = (node: EditableNode) => {
    const full = findDept(deptTree, node.id)
    if (!full) return
    setEditing(full)
    setParent(full.parentId)
    form.reset({ name: full.name, code: full.code, sortOrder: full.sortOrder ?? 0, description: full.description ?? "" })
    setDialogOpen(true)
  }

  const saveMutation = useMutation({
    mutationFn: async (v: DeptForm) => {
      const payload = {
        organizationId: organizationId!,
        parentId: parent,
        name: v.name,
        code: v.code,
        sortOrder: v.sortOrder,
        description: v.description || "",
      }
      if (editing) await departmentsApi.update(editing.id, payload)
      else await departmentsApi.create(payload)
    },
    onSuccess: () => {
      toast.success(editing ? "部门已更新" : "部门创建成功")
      setDialogOpen(false)
      queryClient.invalidateQueries({ queryKey: ["dept-tree"] })
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })

  const [deleting, setDeleting] = React.useState<EditableNode | null>(null)
  const deleteMutation = useMutation({
    mutationFn: (id: string) => departmentsApi.remove(id),
    onSuccess: () => {
      toast.success("部门已删除")
      setDeleting(null)
      queryClient.invalidateQueries({ queryKey: ["dept-tree"] })
    },
    onError: (e: Error) => toast.error(e.message || "删除失败"),
  })

  return (
    <div>
      <PageHeader
        title="部门管理"
        description="组织下的部门树,用于用户归属与数据权限细分"
        icon={<Building2 className="size-5" />}
        actions={
          <div className="flex flex-wrap items-center gap-2">
            <EntityTreePicker
              nodes={orgTree.map((n) => ({ id: n.id, name: n.name, code: n.code, children: toPicker(n.children) }))}
              value={organizationId}
              onSelect={setOrganizationId}
              placeholder="选择组织"
              className="w-52"
              disabled={orgTreeQuery.isLoading}
            />
            <Button onClick={() => openCreate(null)} disabled={!organizationId}>
              <Plus className="size-4" />
              新增根部门
            </Button>
          </div>
        }
      />

      {!organizationId ? (
        <div className="rounded-xl border bg-card">
          <EmptyState
            className="py-16"
            title="请先选择组织"
            description="部门隶属于组织,请在上方选择要管理的组织"
            icon={<Building2 className="size-6" />}
          />
        </div>
      ) : (
        <div className="rounded-xl border bg-card">
          <div className="flex flex-col gap-1 border-b px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <p className="text-sm font-semibold">{selectedOrg?.name ?? "已选组织"} · 部门树</p>
              <p className="text-xs text-muted-foreground">
                {selectedOrg ? `编码 ${selectedOrg.code}` : ""}
              </p>
            </div>
            <Button size="sm" variant="outline" onClick={() => openCreate(null)}>
              <Plus className="size-3.5" />
              新增根部门
            </Button>
          </div>
          <div className="p-3">
            {treeQuery.isLoading ? (
              <div className="space-y-1.5 p-2">
                {Array.from({ length: 5 }).map((_, i) => (
                  <div key={i} className="h-9 animate-pulse rounded-md bg-muted" style={{ marginLeft: `${(i % 3) * 14}px` }} />
                ))}
              </div>
            ) : treeQuery.isError ? (
              <ErrorState
                className="border-0 bg-transparent"
                message={treeQuery.error instanceof Error ? treeQuery.error.message : undefined}
                onRetry={() => treeQuery.refetch()}
              />
            ) : deptTree.length === 0 ? (
              <EmptyState className="py-14" title="该组织暂无部门" description="创建第一个部门,开始构建部门树" />
            ) : (
              <TreeEditor
                nodes={deptToEditable(deptTree)}
                onAdd={openCreate}
                onEdit={openEdit}
                onDelete={setDeleting}
                onAddRoot={() => openCreate(null)}
                addLabel="新增根部门"
              />
            )}
          </div>
        </div>
      )}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{editing ? `编辑部门「${editing.name}」` : "新增部门"}</DialogTitle>
            <DialogDescription>
              {editing ? "" : parent ? "将在所选部门下创建子部门" : "将创建为根部门"}
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form id="dept-form2" onSubmit={form.handleSubmit((v) => saveMutation.mutate(v))} className="space-y-4">
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      部门名称 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: 研发部" {...field} />
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
                      部门编码 <span className="text-destructive">*</span>
                    </FormLabel>
                    <FormControl>
                      <Input placeholder="如: rd" className="font-mono" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
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
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)} disabled={saveMutation.isPending}>
              取消
            </Button>
            <Button type="submit" form="dept-form2" disabled={saveMutation.isPending}>
              {saveMutation.isPending && <Loader2 className="size-4 animate-spin" />}
              {editing ? "保存更改" : "创建"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={!!deleting}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={`删除部门「${deleting?.name ?? ""}」?`}
        description={
          <>
            将<b>级联删除子部门</b>并解除相关用户归属,不可撤销。
          </>
        }
        loading={deleteMutation.isPending}
        onConfirm={() => deleting && deleteMutation.mutate(deleting.id)}
      />
    </div>
  )
}

function toPicker(children?: OrgNode[]): PickerNode[] {
  return (children ?? []).map((c) => ({ id: c.id, name: c.name, code: c.code, children: toPicker(c.children) }))
}
