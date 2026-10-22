import * as React from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useForm } from "react-hook-form"
import { z } from "zod"
import { zResolver } from "@/lib/form"
import { applicationsApi } from "@/api/applications"
import { hostEsmEntryForCode, listHostEsmModules } from "@/micro-apps/registry"
import { toast } from "sonner"
import type { ApplicationRecord } from "@/api/types"

const appSchema = z
  .object({
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
    entryType: z.enum(["ESM", "HTML"]),
    entryUrl: z.string().max(500).optional(),
  })
  .superRefine((v, ctx) => {
    const url = (v.entryUrl ?? "").trim()
    if (url && !/^(https?:\/\/|\/)/i.test(url)) {
      ctx.addIssue({ code: z.ZodIssueCode.custom, path: ["entryUrl"], message: "入口地址需以 http(s):// 或 / 开头" })
    }
  })
type AppForm = z.infer<typeof appSchema>

// 只有 /micro/:appCode/* 会进入微应用容器；宿主内置路由随主前端发布。
export const isMicroApp = (app: ApplicationRecord) =>
  app.basePath?.replace(/\/+$/, "") === `/micro/${app.code}`

// ESM 随宿主构建；只有 HTML 微应用使用独立版本上传。
export const isHostEsm = (app: ApplicationRecord) => isMicroApp(app) && app.entryType === "ESM"


/** Owns create/edit validation, immutable entry metadata and post-save coordination. */
export function useApplicationEditor(onCreatedApplication: (application: ApplicationRecord) => void) {
  const queryClient = useQueryClient()
  const [dialogOpen, setDialogOpen] = React.useState(false)
  const [editing, setEditing] = React.useState<ApplicationRecord | null>(null)
  const [quickSetup, setQuickSetup] = React.useState(true)
  const form = useForm<AppForm>({
    resolver: zResolver(appSchema),
    defaultValues: { code: "", name: "", icon: "LayoutGrid", basePath: "", description: "", sortOrder: 0, status: "ENABLED", entryType: "HTML", entryUrl: "" },
  })
  const entryTypeValue = form.watch("entryType")
  const codeValue = form.watch("code")
  const useQuickSetup = !editing && entryTypeValue === "HTML" && quickSetup
  const hostEsmModules = React.useMemo(() => listHostEsmModules(), [])
  const selectHostEsmModule = (code: string) => {
    const entryUrl = hostEsmEntryForCode(code)
    if (!entryUrl) {
      form.setError("code", { message: "请选择当前宿主中已构建的模块" })
      return
    }
    form.setValue("code", code, { shouldValidate: true })
    form.setValue("entryUrl", entryUrl)
    form.clearErrors("code")
  }

  const openCreate = () => {
    setEditing(null)
    setQuickSetup(true)
    form.reset({ code: "", name: "", icon: "LayoutGrid", basePath: "", description: "", sortOrder: 0, status: "ENABLED", entryType: "HTML", entryUrl: "" })
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
      entryType: a.entryType === "ESM" ? "ESM" : "HTML",
      entryUrl: a.entryUrl ?? "",
    })
    setDialogOpen(true)
  }

  const saveMutation = useMutation({
    mutationFn: async (values: AppForm) => {
      const esm = values.entryType === "ESM"
      const quick = !editing && !esm && quickSetup
      // Metadata editing never rewrites an existing entry or elevates its trust.
      if (editing) {
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
      const esmEntry = esm ? hostEsmEntryForCode(values.code) : null
      if (esm && !esmEntry) throw new Error("请选择当前宿主中已构建的模块")
      const payload = {
        code: values.code,
        name: values.name,
        icon: values.icon || "LayoutGrid",
        basePath: !editing && (quick || esm) ? `/micro/${values.code}` : values.basePath || "",
        description: values.description || "",
        sortOrder: values.sortOrder,
        status: values.status,
        entryType: values.entryType,
        runtimeMode: esm ? "TRUSTED" as const : "IFRAME" as const,
        entryUrl: esm
          ? esmEntry!
          : `/sub-apps/${values.code}/unreleased/index.html`,
        autoCreateMenu: quick,
      }
      const created = await applicationsApi.create(payload)
      return { id: created.id, quick, entryType: values.entryType, application: created }
    },
    onSuccess: (created) => {
      if (created) {
        toast.success("应用已创建", {
          description: created.entryType === "HTML" ? "继续发布版本并配置访问范围" : "模块随宿主发布，继续配置菜单与访问范围",
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


  const submit = form.handleSubmit((values) => {
    if (!editing && values.entryType === "ESM" && !hostEsmEntryForCode(values.code)) {
      form.setError("code", { message: "请选择当前宿主中已构建的模块" })
      return
    }
    saveMutation.mutate(values)
  })
  return { dialogOpen, setDialogOpen, editing, quickSetup, setQuickSetup, form,
    entryTypeValue, codeValue, useQuickSetup, hostEsmModules, selectHostEsmModule,
    openCreate, openEdit, saveMutation, submit }
}
