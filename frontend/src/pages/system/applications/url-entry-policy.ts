import { z } from "zod"

export const urlEntrySchema = z.object({
  entryUrl: z.string().trim().min(1, "请输入 HTTP/HTTPS 入口地址").max(2000)
    .superRefine((value, context) => {
      try {
        const url = new URL(value)
        if (!/^https?:$/.test(url.protocol) || !url.hostname || url.username || url.password
          || /[\s\\]/.test(value) || value.includes("#")
          || [...url.searchParams.keys()].some(key => ["__platform_entry", "__saas_execution", "__saas_lease", "token", "access_token",
            "session", "session_token", "client_secret"].includes(key.toLowerCase()))) {
          throw new Error("invalid URL")
        }
        if (!url.pathname.endsWith("/") && !/\.html?$/i.test(url.pathname)) {
          context.addIssue({ code: "custom", message: "目录地址请以 / 结尾，或填写具体的 .html/.htm 入口" })
        }
      } catch {
        context.addIssue({ code: "custom", message: "请输入不含账号、登录凭据、平台保留参数或锚点的 HTTP/HTTPS 地址" })
      }
    }),
  proxyEnabled: z.boolean(),
})

export type UrlEntryValues = z.infer<typeof urlEntrySchema>
