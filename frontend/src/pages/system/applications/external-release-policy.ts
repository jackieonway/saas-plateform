import { z } from "zod"
import type { ApplicationEnvironment, ApplicationReleasePayload } from "@/api/types"

const previewOrigin = "https://release-preview.invalid"

function releaseUrl(raw: string, relative: boolean) {
  if (!raw || raw.length > 2000 || /\s|\\/.test(raw) || [...raw].some(char => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127) || raw.startsWith("//")) return null
  if (raw.includes("?") || raw.includes("#")) return null
  if (!relative && !raw.startsWith("/") && !raw.startsWith("https://")) return null
  // Reject traversal and encoded separators before URL normalizes the path.
  if (/%(?:2e|2f|5c)/i.test(raw) || raw.split(/[/?#]/).some(part => part === "." || part === "..")) return null
  try {
    const url = new URL(raw, previewOrigin)
    if (url.protocol !== "https:" || url.username || url.password || url.search || url.hash) return null
    return url
  } catch { return null }
}

export const externalReleaseSchema = z.object({
  version: z.string().trim().min(1, "请输入版本号").max(64, "版本号最多 64 个字符")
    .regex(/^[A-Za-z0-9][A-Za-z0-9._-]*$/, "仅支持字母、数字、点、下划线与中划线")
    .refine(value => !value.includes(".."), "版本号不能包含连续的两个点"),
  entryUrl: z.string().trim().min(1, "请输入已部署的 HTML 入口 URL"),
  healthUrl: z.string().trim(),
  resources: z.array(z.object({
    url: z.string().trim().min(1, "请输入资源 URL"),
    kind: z.enum(["SCRIPT", "STYLE", "ASSET"]),
  })).max(48, "显式资源最多 48 项"),
}).superRefine((values, context) => {
  const entry = releaseUrl(values.entryUrl, false)
  if (!entry) context.addIssue({ code: "custom", path: ["entryUrl"], message: "使用根路径或绝对 HTTPS URL，不含凭据、空白、query、fragment 或路径跳转" })
  else if (!entry.pathname.split("/").slice(0, -1).includes(values.version)) {
    context.addIssue({ code: "custom", path: ["entryUrl"], message: "入口路径须包含与版本号完全相同的目录段" })
  }
  if (values.healthUrl && !releaseUrl(values.healthUrl, false)) {
    context.addIssue({ code: "custom", path: ["healthUrl"], message: "健康 URL 使用网关根路径或绝对 HTTPS URL，不含凭据、query 或 fragment" })
  }
  const seen = new Set<string>()
  values.resources.forEach((resource, index) => {
    const parsed = releaseUrl(resource.url, true)
    if (!parsed) {
      context.addIssue({ code: "custom", path: ["resources", index, "url"], message: "使用入口内相对资源路径、根路径或 HTTPS URL，不含路径跳转、query 或 fragment" })
      return
    }
    const resolved = new URL(resource.url, entry ?? previewOrigin).href
    if (seen.has(resolved)) context.addIssue({ code: "custom", path: ["resources", index, "url"], message: "资源 URL 重复" })
    seen.add(resolved)
  })
})

export type ExternalReleaseValues = z.infer<typeof externalReleaseSchema>

export function externalReleasePayload(environment: ApplicationEnvironment, values: ExternalReleaseValues): ApplicationReleasePayload {
  const valid = externalReleaseSchema.parse(values)
  return { environment, version: valid.version, entryType: "HTML", entryUrl: valid.entryUrl,
    healthUrl: valid.healthUrl || null, resources: valid.resources.map(({ url, kind }) => ({ url, kind })) }
}
