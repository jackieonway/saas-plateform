import * as React from "react"
import { AlertTriangle, Inbox } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Table, TableBody, TableCell, TableRow } from "@/components/ui/table"
import { cn } from "@/lib/utils"

/** 空状态 */
export function EmptyState({
  title,
  description,
  icon,
  action,
  className,
}: {
  title: React.ReactNode
  description?: React.ReactNode
  icon?: React.ReactNode
  action?: React.ReactNode
  className?: string
}) {
  return (
    <div className={cn("flex flex-col items-center justify-center gap-3 px-6 py-14 text-center", className)}>
      <div className="flex size-12 items-center justify-center rounded-xl border bg-muted/50 text-muted-foreground">
        {icon ?? <Inbox className="size-6" />}
      </div>
      <div className="space-y-1">
        <p className="text-sm font-medium">{title}</p>
        {description && <p className="text-sm text-muted-foreground">{description}</p>}
      </div>
      {action}
    </div>
  )
}

/** 错误状态 + 重试 */
export function ErrorState({
  message,
  onRetry,
  className,
}: {
  message?: string
  onRetry?: () => void
  className?: string
}) {
  return (
    <div
      role="alert"
      className={cn(
        "flex flex-col items-center justify-center gap-3 rounded-lg border border-destructive/25 bg-destructive/5 px-6 py-10 text-center",
        className
      )}
    >
      <div className="flex size-11 items-center justify-center rounded-xl bg-destructive/10 text-destructive">
        <AlertTriangle className="size-5" />
      </div>
      <div className="space-y-1">
        <p className="text-sm font-medium">数据加载失败</p>
        <p className="mx-auto max-w-md text-sm text-muted-foreground">
          {message || "服务暂时不可用,请稍后重试"}
        </p>
      </div>
      {onRetry && (
        <Button variant="outline" size="sm" onClick={onRetry}>
          重试
        </Button>
      )}
    </div>
  )
}

/** 表格骨架(默认 5 行) */
export function TableSkeleton({ rows = 5, cols = 6 }: { rows?: number; cols?: number }) {
  return (
    <Table>
      <TableBody>
        {Array.from({ length: rows }).map((_, r) => (
          <TableRow key={r} className="pointer-events-none">
            {Array.from({ length: cols }).map((_, c) => (
              <TableCell key={c}>
                <Skeleton className={cn("h-4", c === 0 ? "w-32" : c === cols - 1 ? "w-16" : "w-20")} />
              </TableCell>
            ))}
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}

/** 卡片网格骨架 */
export function CardGridSkeleton({ count = 8, className }: { count?: number; className?: string }) {
  return (
    <div className={cn("grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4", className)}>
      {Array.from({ length: count }).map((_, i) => (
        <div key={i} className="space-y-3 rounded-xl border bg-card p-4">
          <div className="flex items-center gap-3">
            <Skeleton className="size-10 rounded-lg" />
            <div className="flex-1 space-y-1.5">
              <Skeleton className="h-4 w-2/3" />
              <Skeleton className="h-3 w-1/3" />
            </div>
          </div>
          <Skeleton className="h-3 w-full" />
          <Skeleton className="h-3 w-4/5" />
        </div>
      ))}
    </div>
  )
}

/** 统计卡骨架 */
export function StatSkeleton({ count = 4 }: { count?: number }) {
  return (
    <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
      {Array.from({ length: count }).map((_, i) => (
        <div key={i} className="space-y-3 rounded-xl border bg-card p-5">
          <Skeleton className="h-3 w-16" />
          <Skeleton className="h-8 w-24" />
          <Skeleton className="h-3 w-20" />
        </div>
      ))}
    </div>
  )
}

/** 图表骨架 */
export function ChartSkeleton({ className }: { className?: string }) {
  return (
    <div className={cn("flex h-full w-full items-end gap-2 p-4", className)}>
      {[40, 65, 50, 80, 60, 90, 55].map((h, i) => (
        <Skeleton key={i} className="flex-1" style={{ height: `${h}%` }} />
      ))}
    </div>
  )
}

/** 异步区域统一渲染:loading / error / empty / data */
export function AsyncSection<T>({
  isLoading,
  isError,
  error,
  onRetry,
  isEmpty,
  emptyTitle,
  emptyDescription,
  emptyAction,
  emptyIcon,
  skeleton,
  children,
}: {
  isLoading: boolean
  isError: boolean
  error?: unknown
  onRetry?: () => void
  isEmpty: boolean
  emptyTitle?: string
  emptyDescription?: string
  emptyAction?: React.ReactNode
  emptyIcon?: React.ReactNode
  skeleton?: React.ReactNode
  children: React.ReactNode
}) {
  if (isLoading) return <>{skeleton ?? <TableSkeleton />}</>
  if (isError)
    return (
      <ErrorState message={error instanceof Error ? error.message : undefined} onRetry={onRetry} />
    )
  if (isEmpty)
    return (
      <EmptyState
        title={emptyTitle ?? "暂无数据"}
        description={emptyDescription}
        action={emptyAction}
        icon={emptyIcon}
      />
    )
  return <>{children}</>
}
