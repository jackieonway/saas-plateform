import { cn } from "@/lib/utils"

type StatusTone = "success" | "neutral" | "warning" | "danger" | "info"

const TONE_CLASS: Record<StatusTone, string> = {
  success: "border-primary/25 bg-primary/10 text-primary",
  neutral: "border-border bg-muted text-muted-foreground",
  warning: "border-warning/30 bg-warning/10 text-warning",
  danger: "border-destructive/30 bg-destructive/10 text-destructive",
  info: "border-chart-4/30 bg-chart-4/10 text-chart-4",
}

const STATUS_LABEL: Record<string, string> = {
  ENABLED: "启用",
  DISABLED: "停用",
  PENDING: "等待中",
  APPROVED: "已批准",
  DENIED: "已拒绝",
  EXPIRED: "已过期",
  REVOKED: "已吊销",
  UP: "正常",
  DOWN: "异常",
  ACTIVE: "活跃",
}

function toneFor(status: string): StatusTone {
  const s = status.toUpperCase()
  if (["ENABLED", "APPROVED", "UP", "SUCCESS", "ACTIVE"].includes(s)) return "success"
  if (["PENDING", "WAITING"].includes(s)) return "warning"
  if (["DENIED", "REVOKED", "DOWN", "ERROR", "FAILED"].includes(s)) return "danger"
  if (["DISABLED", "EXPIRED", "INACTIVE", "CLOSED"].includes(s)) return "neutral"
  return "info"
}

export function StatusBadge({
  status,
  pulse,
  className,
}: {
  status: string
  /** PENDING 等状态加脉冲动画 */
  pulse?: boolean
  className?: string
}) {
  const tone = toneFor(status)
  const label = STATUS_LABEL[status?.toUpperCase()] ?? status
  const animate = pulse || status?.toUpperCase() === "PENDING"
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs font-medium whitespace-nowrap",
        TONE_CLASS[tone],
        className
      )}
    >
      <span
        className={cn(
          "size-1.5 rounded-full bg-current",
          animate && "animate-pulse"
        )}
      />
      {label}
    </span>
  )
}

/** 带文本的通用类型徽章(outline 风格) */
export function TypeBadge({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-md border bg-background px-1.5 py-0.5 text-xs font-medium text-muted-foreground",
        className
      )}
    >
      {children}
    </span>
  )
}
