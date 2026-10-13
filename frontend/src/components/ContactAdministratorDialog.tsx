import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { CopyButton } from "@/components/CopyButton"
import type { ContactAdministrator } from "@/api/types"

export function ContactAdministratorDialog({ administrator, onClose }: { administrator: ContactAdministrator | null; onClose: () => void }) {
  return <Dialog open={!!administrator} onOpenChange={(open) => { if (!open) onClose() }}>
    <DialogContent className="sm:max-w-md">
      <DialogHeader>
        <DialogTitle>{administrator?.alreadyInitialized ? "首位管理员已初始化" : "首位管理员已开通"}</DialogTitle>
        <DialogDescription>{administrator?.alreadyInitialized ? "已有管理员，未创建新人员或修改密码、授权。" : administrator?.created ? "请保存登录信息，初始密码仅在本次创建后显示。" : "已关联联系人现有账号，使用原密码登录。"}</DialogDescription>
      </DialogHeader>
      <div className="space-y-4">
        <div className="space-y-2"><Label htmlFor="contact-admin-username">登录用户名</Label>
          <div className="flex gap-2"><Input id="contact-admin-username" value={administrator?.username ?? ""} readOnly /><CopyButton text={administrator?.username ?? ""} size="icon" /></div>
        </div>
        {administrator?.initialPassword && <div className="space-y-2"><Label htmlFor="contact-admin-password">初始密码</Label>
          <div className="flex gap-2"><Input id="contact-admin-password" value={administrator.initialPassword} readOnly /><CopyButton text={administrator.initialPassword} size="icon" /></div>
        </div>}
        <p className="text-sm text-muted-foreground">{administrator?.alreadyInitialized ? "当前管理员持有" : "已授予"}「{administrator?.roleName}」角色，可使用电话或 Email 登录。</p>
      </div>
      <DialogFooter><Button onClick={onClose}>已保存，关闭</Button></DialogFooter>
    </DialogContent>
  </Dialog>
}
