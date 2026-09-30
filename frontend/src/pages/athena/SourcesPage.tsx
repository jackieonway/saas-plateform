import { Database } from "lucide-react"
import { Card, CardContent } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { PageHeader } from "@/components/layout/PageHeader"
import { DemoBadge, SOURCE_TYPE_META, useAthenaSources } from "@/pages/athena/mock"
import { AppIcon } from "@/lib/icons"
import { formatRelative } from "@/lib/format"
import { cn } from "@/lib/utils"

export default function SourcesPage() {
  const sources = useAthenaSources()

  return (
    <div className="space-y-6">
      <PageHeader
        title="数据源"
        description="Athena 数据分析 · 接入的数据源与同步状态(演示数据)"
        icon={<Database className="size-5" />}
        badge={<DemoBadge />}
      />

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
        {sources.map((s) => {
          const meta = SOURCE_TYPE_META[s.type]
          return (
            <Card key={s.name} className="transition-shadow hover:shadow-md">
              <CardContent className="p-5">
                <div className="mb-3 flex items-start justify-between gap-2">
                  <div className="flex items-center gap-3">
                    <span className={cn("flex size-11 items-center justify-center rounded-xl", meta.className)}>
                      <AppIcon name={meta.icon} className="size-6" />
                    </span>
                    <div className="min-w-0">
                      <p className="truncate font-semibold">{s.name}</p>
                      <p className="text-xs text-muted-foreground">{s.type}</p>
                    </div>
                  </div>
                  <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                    <span
                      className={cn(
                        "size-2 rounded-full",
                        s.status === "UP" ? "bg-primary" : s.status === "PENDING" ? "bg-warning animate-pulse" : "bg-destructive"
                      )}
                    />
                    {s.status === "UP" ? "正常" : s.status === "PENDING" ? "同步中" : "异常"}
                  </span>
                </div>
                <code className="block truncate rounded-md bg-muted px-2 py-1.5 font-mono text-xs text-muted-foreground">
                  {s.host}
                </code>
                <div className="mt-3 flex items-center justify-between text-xs text-muted-foreground">
                  <span>延迟 {s.latencyMs}ms</span>
                  <span>{s.tables > 0 ? `${s.tables} 张表` : "—"}</span>
                  <span>同步于 {formatRelative(s.lastSyncAt)}</span>
                </div>
              </CardContent>
            </Card>
          )
        })}
      </div>
    </div>
  )
}
