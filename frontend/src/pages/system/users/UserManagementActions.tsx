import { UserPlus } from "lucide-react"
import { Button } from "@/components/ui/button"

export function UserManagementActions({ memberType, createMembershipBlockedReason, onAddMembership }: {
  memberType: "EMPLOYEE" | "CUSTOMER"
  createMembershipBlockedReason: string | null | undefined
  onAddMembership: () => void
}) {
  return <div className="flex gap-2">
    <Button disabled={!!createMembershipBlockedReason} title={createMembershipBlockedReason ?? undefined} onClick={onAddMembership}>
      <UserPlus className="size-4" />{memberType === "EMPLOYEE" ? "新建员工" : "新建客户"}
    </Button>
  </div>
}
