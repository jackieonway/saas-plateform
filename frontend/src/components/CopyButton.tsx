import * as React from "react"
import { Check, Copy } from "lucide-react"
import { Button } from "@/components/ui/button"
import { copyText } from "@/lib/format"
import { toast } from "sonner"
import { cn } from "@/lib/utils"

export function CopyButton({
  text,
  label,
  className,
  size = "sm",
}: {
  text: string
  label?: string
  className?: string
  size?: "sm" | "default" | "icon"
}) {
  const [copied, setCopied] = React.useState(false)
  return (
    <Button
      type="button"
      variant="outline"
      size={size}
      className={className}
      aria-label="复制"
      onClick={async () => {
        const ok = await copyText(text)
        if (ok) {
          setCopied(true)
          toast.success("已复制到剪贴板")
          setTimeout(() => setCopied(false), 1600)
        } else {
          toast.error("复制失败,请手动复制")
        }
      }}
    >
      {copied ? <Check className="size-3.5 text-primary" /> : <Copy className="size-3.5" />}
      {label && <span>{label}</span>}
    </Button>
  )
}

/** 一次性密钥展示弹窗 */
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { AlertTriangle } from "lucide-react"

export function SecretDialog({
  open,
  onOpenChange,
  secret,
  title = "客户端密钥",
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  secret: string
  title?: string
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>请立即保存,该密钥仅此一次展示,关闭后无法再次查看。</DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="rounded-lg border bg-muted/50 p-3">
            <code className={cn("block break-all font-mono text-sm", secret.length > 48 && "text-xs")}>{secret}</code>
          </div>
          <div className="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm text-warning">
            <AlertTriangle className="mt-0.5 size-4 shrink-0" />
            <span>仅此一次展示:请妥善保管该密钥,遗忘后需重新生成。</span>
          </div>
        </div>
        <DialogFooter>
          <CopyButton text={secret} label="复制密钥" />
          <Button onClick={() => onOpenChange(false)}>我已保存</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
