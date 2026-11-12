import type { UserPayload } from "@/api/types"
import { isUuidV7 } from "@/lib/uuid"

export type NewMemberAccount = { username: string; password: string; realName: string; phone: string; email: string }

export function newMemberPayload(account: NewMemberAccount, organizationId: string, memberType: "EMPLOYEE" | "CUSTOMER", position: string, reason: string): UserPayload {
  const phone = account.phone.replace(/[\s()-]/g, "")
  if (!phone) throw new Error("电话必填")
  if (account.phone.length > 30 || !/^\+?\d+$/.test(phone)) throw new Error("电话号码格式不正确")
  const email = account.email.trim()
  if (email && !/^[^\s@]+@[^\s@]+$/.test(email)) throw new Error("邮箱格式不正确")
  if (!isUuidV7(organizationId)) throw new Error("请选择目标组织")
  return {
    username: account.username.trim(), password: account.password, realName: account.realName.trim(),
    phone: account.phone.trim(), email, gender: "UNKNOWN",
    membership: { organizationId, memberType, position: memberType === "EMPLOYEE" ? position.trim() : "", reason: reason.trim() || `新建${memberType === "EMPLOYEE" ? "员工" : "客户"}` },
  }
}
