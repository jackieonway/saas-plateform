import { TrendingUp } from "lucide-react"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { PageHeader } from "@/components/layout/PageHeader"
import { DemoBadge, useAthenaOperations } from "@/pages/athena/mock"
import { formatNumber } from "@/lib/format"
import { cn } from "@/lib/utils"

export default function OperationsReportPage() {
  const { funnel, table } = useAthenaOperations()
  const max = funnel[0].count

  return (
    <div className="space-y-6">
      <PageHeader
        title="运营报表"
        description="Athena 数据分析 · 转化漏斗与区域经营概览(演示数据)"
        icon={<TrendingUp className="size-5" />}
        badge={<DemoBadge />}
      />

      {/* 漏斗 */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">转化漏斗</CardTitle>
          <CardDescription>访问 → 支付的全链路转化(近 30 日)</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {funnel.map((f, i) => {
            const width = (f.count / max) * 100
            const prev = i > 0 ? funnel[i - 1] : null
            const stepConv = prev ? (f.count / prev.count) * 100 : 100
            return (
              <div key={f.stage} className="space-y-1.5">
                <div className="flex items-baseline justify-between gap-3 text-sm">
                  <span className="font-medium">
                    <span className="mr-2 inline-flex size-5 items-center justify-center rounded-md bg-muted font-mono text-[10px] text-muted-foreground">
                      {i + 1}
                    </span>
                    {f.stage}
                  </span>
                  <span className="text-xs text-muted-foreground tabular-nums">
                    {formatNumber(f.count)} 人 · 总转化 {f.pct}%
                    {prev && <span className="ml-2 text-primary">环节转化 {stepConv.toFixed(1)}%</span>}
                  </span>
                </div>
                <div className="h-8 w-full overflow-hidden rounded-lg bg-muted/60">
                  <div
                    className={cn(
                      "flex h-full items-center justify-end rounded-lg pr-2 text-xs font-medium text-primary-foreground transition-all",
                      i === 0 ? "bg-primary" : i < 3 ? "bg-primary/80" : "bg-primary/60"
                    )}
                    style={{ width: `${width}%` }}
                  >
                    {f.pct}%
                  </div>
                </div>
              </div>
            )
          })}
        </CardContent>
      </Card>

      {/* 区域表 */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">区域经营概览</CardTitle>
          <CardDescription>各大区订单、GMV 与转化表现</CardDescription>
        </CardHeader>
        <CardContent className="px-0 pb-0">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-6 text-xs font-medium uppercase tracking-wider text-muted-foreground">区域</TableHead>
                <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">订单量</TableHead>
                <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">GMV(元)</TableHead>
                <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">转化率</TableHead>
                <TableHead className="pr-6 text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">满意度</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {table.map((r) => (
                <TableRow key={r.region}>
                  <TableCell className="pl-6 text-sm font-medium">{r.region}</TableCell>
                  <TableCell className="text-right tabular-nums">{formatNumber(r.orders)}</TableCell>
                  <TableCell className="text-right tabular-nums">{formatNumber(Math.round(r.gmv))}</TableCell>
                  <TableCell className="text-right tabular-nums">{r.conversion}</TableCell>
                  <TableCell className="pr-6 text-right tabular-nums">{r.satisfaction} / 100</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  )
}
