import * as React from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import { applicationsApi } from "@/api/applications"
import { toast } from "sonner"
import type { ApplicationRecord } from "@/api/types"

const appSchema = z.object({
    code: z
      .string()
      .min(2, "请输入应用编码")
      .max(32)
      .regex(/^[a-z0-9-]+$/, "仅支持小写字母、数字与中划线"),
    name: z.string().min(1, "请输入应用名称").max(64),
    icon: z.string().max(64).optional(),
    basePath: z.string().max(128).optional(),
    description: z.string().max(255).optional(),
    sortOrder: z.coerce.number().int().min(0).max(9999),
    status: z.enum(["ENABLED", "DISABLED"]),
  })
type AppForm = z.infer<typeof appSchema>

// 只有 /micro/:appCode/* 会进入微应用容器；宿主内置路由随主前端发布。
export const isMicroApp = (app: ApplicationRecord) =>
  app.basePath?.replace(/\/+$/, "") === `/micro/${app.code}`


/** Owns create/edit validation, immutable entry metadata and post-save coordination. */
export function useApplicationEditor(onCreatedApplication: (application: ApplicationRecord) => void) {
  const queryClient = useQueryClient()
  const [dialogOpen, setDialogOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<ApplicationRecord | null>(null)
  const [quickSetup, setQuickSetup] = React.useState(true)
  const form = useForm<AppForm>({
    resolver: zResolver(appSchema),
    defaultValues: { code: "", name: "", icon: "LayoutGrid", basePath: "", description: "", sortOrder: 0, status: "ENABLED" },
  })
  const codeValue = form.watch("code")
  const useQuickSetup = !editing && quickSetup

  const openCreate = () => {
    setEditing(null)
    setQuickSetup(true)
    form.reset({ code: "", name: "", icon: "LayoutGrid", basePath: "", description: "", sortOrder: 0, status: "ENABLED" })
    setDialogOpen(true)
  }
  const openEdit = (a: ApplicationRecord) => {
    setEditing(a)
    setQuickSetup(false)
    form.reset({
      code: a.code,
      name: a.name,
      icon: a.icon ?? "LayoutGrid",
      basePath: a.basePath ?? "",
      description: a.description ?? "",
      sortOrder: a.sortOrder ?? 0,
      status: a.status,
    })
    setDialogOpen(true)
  }

  const saveMutation = useMutation({
    mutationFn: async (values: AppForm) => {
      const quick = !editing && quickSetup
      // Entry configuration is managed separately from application metadata.
      if (editing) {
        if (isMicroApp(editing) && (editing.entryType !== "HTML" || editing.runtimeMode !== "TRUSTED")) {
          throw new Error("请先在应用入口中重新配置 qiankun，再编辑或启用应用")
        }
        await applicationsApi.update(editing.id, {
          code: editing.code,
          name: values.name,
          icon: values.icon || "LayoutGrid",
          basePath: values.basePath || "",
          description: values.description || "",
          sortOrder: values.sortOrder,
          status: values.status,
        })
        return null
      }
      const payload = {
        code: values.code,
        name: values.name,
        icon: values.icon || "LayoutGrid",
        basePath: quick ? `/micro/${values.code}` : values.basePath || "",
        description: values.description || "",
        sortOrder: values.sortOrder,
        status: values.status,
        entryType: "HTML" as const,
        runtimeMode: "TRUSTED" as const,
        entryUrl: `/sub-apps/${values.code}/unreleased/index.html`,
        autoCreateMenu: quick,
      }
      const created = await applicationsApi.create(payload)
      return { id: created.id, quick, application: created }
    },
    onSuccess: (created) => {
      if (created) {
        toast.success("应用已创建", {
          description: "继续配置 qiankun 应用入口与访问范围",
        })
        onCreatedApplication(created.application)
      } else {
        toast.success(editing ? "应用已更新" : "应用创建成功")
      }
      setDialogOpen(false)
      queryClient.invalidateQueries({ queryKey: ["applications"] })
      queryClient.invalidateQueries({ queryKey: ["profile"] })
    },
    onError: (e: Error) => toast.error(e.message || "保存失败"),
  })


  const submit = form.handleSubmit((values) => saveMutation.mutate(values))
  return { dialogOpen, setDialogOpen, editing, quickSetup, setQuickSetup, form,
    codeValue, useQuickSetup,
    openCreate, openEdit, saveMutation, submit }
}
