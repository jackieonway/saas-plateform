import { Plus, UserPlus } from "lucide-react"
import { Button } from "@/components/ui/button"

export function UserManagementActions({ memberType, createMembershipBlockedReason, onCreateAccount, onAddMembership }: {
  memberType: "EMPLOYEE" | "CUSTOMER"
  createMembershipBlockedReason: string | null | undefined
  onCreateAccount: () => void
  onAddMembership: () => void
}) {
  return <div className="flex gap-2">
    <Button variant="outline" onClick={onCreateAccount}>
      <Plus className="size-4" />新增账号
    </Button>
    <Button disabled={!!createMembershipBlockedReason} title={createMembershipBlockedReason ?? undefined} onClick={onAddMembership}>
      <UserPlus className="size-4" />{memberType === "EMPLOYEE" ? "添加员工" : "添加客户"}
    </Button>
  </div>
}
