import * as React from "react"
import { CalendarDays } from "lucide-react"
import {
  Bar,
  BarChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip as ReTooltip,
  XAxis,
  YAxis,
} from "recharts"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { Table, TableBody, TableCell, TableFooter, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { PageHeader } from "@/components/layout/PageHeader"
import { DemoBadge, useAthenaSales } from "@/pages/athena/mock"
import { formatNumber } from "@/lib/format"

function Tip({ active, payload, label }: { active?: boolean; payload?: { name?: string; value?: number }[]; label?: string }) {
  if (!active || !payload?.length) return null
  return (
    <div className="rounded-lg border bg-popover px-3 py-2 text-xs shadow-lg">
      <p className="mb-1 font-medium">{label}</p>
      {payload.map((p, i) => (
        <p key={i} className="flex justify-between gap-4 tabular-nums text-muted-foreground">
          <span>{p.name}</span>
          <span className="font-semibold text-foreground">{formatNumber(p.value)}</span>
        </p>
      ))}
    </div>
  )
}

const STATUS_TONE: Record<string, string> = {
  已完成: "border-primary/25 bg-primary/10 text-primary",
  进行中: "border-warning/30 bg-warning/10 text-warning",
  待支付: "border-border bg-muted text-muted-foreground",
}

export default function SalesReportPage() {
  const [range, setRange] = React.useState("近 30 天")
  const { byCategory, details, totalAmount, totalCount } = useAthenaSales(range)

  return (
    <div className="space-y-6">
      <PageHeader
        title="销售报表"
        description="Athena 数据分析 · 按品类汇总的销售表现(演示数据)"
        icon={<CalendarDays className="size-5" />}
        badge={<DemoBadge />}
        actions={
          <Select value={range} onValueChange={setRange}>
            <SelectTrigger className="w-36" aria-label="日期范围">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="近 7 天">近 7 天</SelectItem>
              <SelectItem value="近 30 天">近 30 天</SelectItem>
              <SelectItem value="近 90 天">近 90 天</SelectItem>
            </SelectContent>
          </Select>
        }
      />

      {/* 分组柱状图 */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">品类销售额与订单量({range})</CardTitle>
          <CardDescription>左轴销售额(元)· 右轴订单量(单)</CardDescription>
        </CardHeader>
        <CardContent className="h-80">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={byCategory} margin={{ top: 8, right: 8, left: -8, bottom: 0 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" vertical={false} />
              <XAxis dataKey="category" tick={{ fontSize: 12, fill: "var(--muted-foreground)" }} tickLine={false} axisLine={false} />
              <YAxis yAxisId="l" tick={{ fontSize: 11, fill: "var(--muted-foreground)" }} tickLine={false} axisLine={false} width={64} />
              <YAxis yAxisId="r" orientation="right" tick={{ fontSize: 11, fill: "var(--muted-foreground)" }} tickLine={false} axisLine={false} width={48} />
              <ReTooltip content={<Tip />} cursor={{ fill: "var(--muted)", opacity: 0.4 }} />
              <Legend wrapperStyle={{ fontSize: 12 }} />
              <Bar yAxisId="l" dataKey="销售额" fill="var(--chart-1)" radius={[4, 4, 0, 0]} maxBarSize={36} />
              <Bar yAxisId="r" dataKey="订单量" fill="var(--chart-2)" radius={[4, 4, 0, 0]} maxBarSize={36} />
            </BarChart>
          </ResponsiveContainer>
        </CardContent>
      </Card>

      {/* 明细表 */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">重点订单明细</CardTitle>
          <CardDescription>近期代表性订单与状态</CardDescription>
        </CardHeader>
        <CardContent className="px-0 pb-0">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-6 text-xs font-medium uppercase tracking-wider text-muted-foreground">订单号</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">客户</TableHead>
                <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">金额(元)</TableHead>
                <TableHead className="text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">件数</TableHead>
                <TableHead className="text-xs font-medium uppercase tracking-wider text-muted-foreground">状态</TableHead>
                <TableHead className="pr-6 text-right text-xs font-medium uppercase tracking-wider text-muted-foreground">日期</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {details.map((d) => (
                <TableRow key={d.id}>
                  <TableCell className="pl-6 font-mono text-xs font-medium">{d.id}</TableCell>
                  <TableCell className="text-sm">{d.customer}</TableCell>
                  <TableCell className="text-right tabular-nums">{formatNumber(Math.round(d.amount))}</TableCell>
                  <TableCell className="text-right tabular-nums">{d.count}</TableCell>
                  <TableCell>
                    <span className={`inline-flex rounded-md border px-2 py-0.5 text-xs ${STATUS_TONE[d.status]}`}>{d.status}</span>
                  </TableCell>
                  <TableCell className="pr-6 text-right text-xs text-muted-foreground">{d.date}</TableCell>
                </TableRow>
              ))}
            </TableBody>
            <TableFooter>
              <TableRow>
                <TableCell className="pl-6 text-sm font-semibold" colSpan={2}>
                  汇总
                </TableCell>
                <TableCell className="text-right text-sm font-semibold tabular-nums">{formatNumber(Math.round(totalAmount))}</TableCell>
                <TableCell className="text-right text-sm font-semibold tabular-nums">{formatNumber(totalCount)}</TableCell>
                <TableCell colSpan={2} />
              </TableRow>
            </TableFooter>
          </Table>
        </CardContent>
      </Card>
    </div>
  )
}
