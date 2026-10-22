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
import { EntityTreePicker } from "@/components/EntityTreePicker"
import { EmptyState, ErrorState } from "@/components/states"
import { departmentsApi } from "@/api/departments"
import { asArray } from "@/lib/api"
import { useAuthStore } from "@/stores/auth"
import { toast } from "sonner"
import type { DepartmentOrgNode, DeptNode } from "@/api/types"

const deptSchema = z.object({
  name: z.string().min(1, "请输入部门名称").max(64),
  code: z.string().min(1, "请输入编码").max(32).regex(/^[a-zA-Z0-9-]+$/, "仅支持字母、数字与中划线"),
  sortOrder: z.coerce.number().int().min(0).max(9999),
  description: z.string().max(255).optional(),
})
type DeptForm = z.infer<typeof deptSchema>
type DepartmentSaveRequest = {
  values: DeptForm
  organizationId: string
  parentId: string | null
  editingId: string | null
  viewKey: string
}
type DepartmentDeleteRequest = { id: string; organizationId: string; viewKey: string }

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

function findOrganization(nodes: DepartmentOrgNode[], id: string): DepartmentOrgNode | null {
  for (const node of nodes) {
    if (node.id === id) return node
    const child = findOrganization(node.children ?? [], id)
    if (child) return child
  }
  return null
}

export default function DepartmentsPage() {
  const queryClient = useQueryClient()
  const profile = useAuthStore((s) => s.profile)
  const contextEpoch = useAuthStore((s) => s.contextEpoch)
  const contextSwitching = useAuthStore((s) => s.contextSwitching)
  const tenantId = profile?.tenant.id ?? null
  const defaultOrganizationId = profile?.organization.id ?? null
  const contextKey = `${contextEpoch}:${tenantId}:${defaultOrganizationId}`
  const contextReady = !!tenantId && !!defaultOrganizationId && !contextSwitching
  const [selection, setSelection] = React.useState({ contextKey, id: defaultOrganizationId })

  const orgTreeQuery = useQuery({
    queryKey: ["department-organizations", tenantId, defaultOrganizationId, contextEpoch],
    queryFn: async ({ signal }) => asArray<DepartmentOrgNode>(await departmentsApi.organizations(signal)),
    enabled: contextReady,
    staleTime: 30_000,
    retry: 1,
  })
  const orgTree = React.useMemo(() => {
    if (!contextReady || !profile) return []
    const current = findOrganization(orgTreeQuery.data ?? [], profile.organization.id)
    // The authenticated profile remains authoritative while the selector loads
    // or is unavailable; never fall back to an unrelated organization.
    return [current ?? { id: profile.organization.id, name: profile.organization.name, code: "", children: [] }]
  }, [contextReady, profile, orgTreeQuery.data])
  const selectedOrg = contextReady
    ? findOrganization(orgTree, selection.contextKey === contextKey && selection.id ? selection.id : defaultOrganizationId!)
      ?? orgTree[0]
    : null
  // Derive the active organization during render so session switches cannot issue a request for the
  // previously selected organization before a state-reset effect runs.
  const organizationId = selectedOrg?.id ?? null
  const scopeKey = `${contextKey}:${organizationId}`
  const scopeGeneration = React.useRef({ key: scopeKey, value: 0 })
  if (scopeGeneration.current.key !== scopeKey) scopeGeneration.current = { key: scopeKey, value: scopeGeneration.current.value + 1 }
  const viewKey = `${scopeKey}:${scopeGeneration.current.value}`
  const activeView = React.useRef(viewKey)
  activeView.current = viewKey
  const mounted = React.useRef(true)
  const writing = React.useRef<string | null>(null)
  React.useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  const isCurrent = (key: string) => mounted.current && activeView.current === key
  const finishWrite = (key: string) => { if (writing.current === key) writing.current = null }

  const treeQuery = useQuery({
    queryKey: ["dept-tree", organizationId, contextEpoch],
    queryFn: async ({ signal }) => asArray<DeptNode>(await departmentsApi.tree(organizationId!, signal)),
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
    if (!organizationId || !isCurrent(viewKey) || writing.current === viewKey) return
    setEditing(null)
    setParent(parentId)
    form.reset({ name: "", code: "", sortOrder: 0, description: "" })
    setDialogOpen(true)
  }
  const openEdit = (node: EditableNode) => {
    if (!isCurrent(viewKey) || writing.current === viewKey) return
    const full = findDept(deptTree, node.id)
    if (!full) return
    setEditing(full)
    setParent(full.parentId)
    form.reset({ name: full.name, code: full.code, sortOrder: full.sortOrder ?? 0, description: full.description ?? "" })
    setDialogOpen(true)
  }

  const saveMutation = useMutation({
    mutationFn: async (request: DepartmentSaveRequest) => {
      if (!isCurrent(request.viewKey) || contextSwitching) throw new DOMException("部门管理范围已变化", "AbortError")
      const v = request.values
      const payload = {
        organizationId: request.organizationId,
        parentId: request.parentId,
        name: v.name,
        code: v.code,
        sortOrder: v.sortOrder,
        description: v.description || "",
      }
      if (request.editingId) await departmentsApi.update(request.editingId, payload)
      else await departmentsApi.create(payload)
    },
    onSuccess: (_result, request) => {
      finishWrite(request.viewKey)
      queryClient.invalidateQueries({ queryKey: ["dept-tree", request.organizationId] })
      if (!isCurrent(request.viewKey)) return
      toast.success(request.editingId ? "部门已更新" : "部门创建成功")
      setDialogOpen(false)
    },
    onError: (e: Error, request) => {
      finishWrite(request.viewKey)
      if (isCurrent(request.viewKey) && e.name !== "AbortError") toast.error(e.message || "保存失败")
    },
  })

  const [deleting, setDeleting] = React.useState<EditableNode | null>(null)
  React.useEffect(() => {
    setDialogOpen(false)
    setEditing(null)
    setParent(null)
    setDeleting(null)
  }, [contextKey, organizationId])

  const deleteMutation = useMutation({
    mutationFn: (request: DepartmentDeleteRequest) => {
      if (!isCurrent(request.viewKey)) throw new DOMException("部门管理范围已变化", "AbortError")
      return departmentsApi.remove(request.id)
    },
    onSuccess: (_result, request) => {
      finishWrite(request.viewKey)
      queryClient.invalidateQueries({ queryKey: ["dept-tree", request.organizationId] })
      if (!isCurrent(request.viewKey)) return
      toast.success("部门已删除")
      setDeleting(null)
    },
    onError: (e: Error, request) => {
      finishWrite(request.viewKey)
      if (isCurrent(request.viewKey) && e.name !== "AbortError") toast.error(e.message || "删除失败")
    },
  })

  return (
    <div>
      <PageHeader
        title="部门管理"
        description="默认管理当前登录组织的部门，可切换到下级组织"
        icon={<Building2 className="size-5" />}
        actions={
          <div className="flex flex-wrap items-center gap-2">
            <EntityTreePicker
              nodes={orgTree}
              value={organizationId}
              onSelect={(id) => {
                if (id && findOrganization(orgTree, id)) setSelection({ contextKey, id })
              }}
              placeholder="选择组织"
              className="w-52"
              allowClear={false}
              disabled={!contextReady || orgTreeQuery.isLoading}
            />
            <Button onClick={() => openCreate(null)} disabled={!organizationId}>
              <Plus className="size-4" />
              新增根部门
            </Button>
          </div>
        }
      />

      {contextReady && orgTreeQuery.isError && (
        <ErrorState
          className="mb-4"
          message="下级组织加载失败，当前登录组织的部门仍可管理。"
          onRetry={() => orgTreeQuery.refetch()}
        />
      )}

      {!organizationId ? (
        <div className="rounded-xl border bg-card">
          <EmptyState
            className="py-16"
            title="正在加载登录组织"
            description="登录组织加载完成后，将自动展示该组织的部门"
            icon={<Building2 className="size-6" />}
          />
        </div>
      ) : (
        <div className="rounded-xl border bg-card">
          <div className="flex flex-col gap-1 border-b px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <p className="text-sm font-semibold">{selectedOrg?.name ?? "已选组织"} · 部门树</p>
              <p className="text-xs text-muted-foreground">
                {selectedOrg?.code ? `编码 ${selectedOrg.code}` : ""}
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

      <Dialog open={dialogOpen} onOpenChange={value => { if (writing.current !== viewKey) setDialogOpen(value) }}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{editing ? `编辑部门「${editing.name}」` : "新增部门"}</DialogTitle>
            <DialogDescription>
              {editing ? "" : parent ? "将在所选部门下创建子部门" : "将创建为根部门"}
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form id="dept-form2" onSubmit={form.handleSubmit((values) => {
              if (!organizationId || !contextReady || !isCurrent(viewKey) || writing.current === viewKey) return
              writing.current = viewKey
              saveMutation.mutate({ values, organizationId, parentId: parent, editingId: editing?.id ?? null, viewKey })
            })} className="space-y-4">
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
        onConfirm={() => {
          if (!deleting || !organizationId || !contextReady || !isCurrent(viewKey) || writing.current === viewKey) return
          writing.current = viewKey
          deleteMutation.mutate({ id: deleting.id, organizationId, viewKey })
        }}
      />
    </div>
  )
}
