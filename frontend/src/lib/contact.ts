import { z } from "zod"

export const telephoneSchema = z.string().trim().max(30, "电话最多 30 个字符")
  .refine((value) => !value || /^\+?\d{6,30}$/.test(value.replace(/[\s()-]/g, "")), "电话格式不正确")

export const contactFields = {
  contactName: z.string().trim().max(100, "联系人姓名最多 100 个字符"),
  contactPhone: telephoneSchema,
  contactEmail: z.string().trim().max(100, "Email 最多 100 个字符").email("Email 格式不正确").or(z.literal("")),
}

export function requireContact(values: { contactName: string; contactPhone: string; contactEmail: string }, ctx: z.RefinementCtx) {
  if (!values.contactName) ctx.addIssue({ code: "custom", path: ["contactName"], message: "请输入联系人姓名" })
  if (!values.contactPhone && !values.contactEmail) {
    ctx.addIssue({ code: "custom", path: ["contactPhone"], message: "请至少填写电话或 Email" })
  }
}
