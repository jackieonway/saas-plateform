import * as React from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import { UserRoundPlus } from "lucide-react"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import type { ContactAdministrator, EntityCreationResult } from "@/api/types"

export function InitializeAdministratorButton({ name, initialize, onSuccess }: {
  name: string
  initialize: () => Promise<EntityCreationResult>
  onSuccess: (administrator: ContactAdministrator) => void
}) {
  const [open, setOpen] = React.useState(false)
  const writing = React.useRef(false)
  const mounted = React.useRef(true)
  React.useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  const queryClient = useQueryClient()
  const mutation = useMutation({
    mutationFn: initialize,
    onSuccess: (result) => {
      writing.current = false
      for (const key of ["tenants", "org-tree", "users", "roles", "profile"]) {
        queryClient.invalidateQueries({ queryKey: [key] })
      }
      if (!mounted.current) return
      setOpen(false)
      if (result.administrator) onSuccess(result.administrator)
    },
    onError: (error: Error) => { writing.current = false; if (mounted.current && error.name !== "AbortError") toast.error(error.message || "初始化失败") },
  })
  return <>
    <Tooltip>
      <TooltipTrigger asChild>
        <Button size="icon" variant="ghost" className="size-8 shrink-0" aria-label="初始化首位管理员" disabled={mutation.isPending} onClick={() => { if (!writing.current) setOpen(true) }}>
          <UserRoundPlus className="size-4" aria-hidden="true" />
        </Button>
      </TooltipTrigger>
      <TooltipContent>初始化首位管理员</TooltipContent>
    </Tooltip>
    <Dialog open={open} onOpenChange={(value) => { if (!writing.current) setOpen(value) }}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>初始化首位管理员</DialogTitle>
          <DialogDescription>为「{name}」使用已保存的联系人信息初始化管理员。请先保存联系人姓名，以及电话或 Email。</DialogDescription>
        </DialogHeader>
        <p className="text-sm text-muted-foreground">已有管理员时会保留现有人员和授权；没有管理员时，匹配联系人现有账号或创建新账号，再授予管理员角色。现有账号保留原密码，新账号初始密码仅显示一次。</p>
        <DialogFooter>
          <Button variant="outline" disabled={mutation.isPending} onClick={() => setOpen(false)}>取消</Button>
          <Button disabled={mutation.isPending} onClick={() => { if (!mounted.current || writing.current) return; writing.current = true; mutation.mutate() }}>{mutation.isPending ? "正在初始化…" : "确认初始化"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </>
}
