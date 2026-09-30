import { BarChart3 } from "lucide-react"
import {
  Area,
  AreaChart,
  CartesianGrid,
  Cell,
  Legend,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip as ReTooltip,
  XAxis,
  YAxis,
} from "recharts"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { Skeleton } from "@/components/ui/skeleton"
import { PageHeader } from "@/components/layout/PageHeader"
import { DemoBadge, useAthenaOverview } from "@/pages/athena/mock"
import { formatNumber } from "@/lib/format"
import { cn } from "@/lib/utils"
import { TrendingDown, TrendingUp } from "lucide-react"

const PIE_COLORS = ["var(--chart-1)", "var(--chart-2)", "var(--chart-3)", "var(--chart-4)", "var(--chart-5)", "var(--muted-foreground)"]

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

export default function AthenaOverviewPage() {
  const { kpis, compare, channels } = useAthenaOverview()

  return (
    <div className="space-y-6">
      <PageHeader
        title="数据总览"
        description="Athena 数据分析 · 核心指标与流量趋势(演示数据)"
        icon={<BarChart3 className="size-5" />}
        badge={<DemoBadge />}
      />

      {/* KPI 卡 */}
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {kpis.map((k) => {
          const up = k.delta >= 0
          return (
            <Card key={k.label} className="relative overflow-hidden">
              <div aria-hidden className="absolute -right-5 -top-5 flex size-20 items-center justify-center rounded-full bg-primary/8 text-primary/10">
                <BarChart3 className="size-10" />
              </div>
              <CardHeader className="pb-2">
                <CardDescription>{k.label}</CardDescription>
                <CardTitle className="text-2xl font-bold tabular-nums">{k.value}</CardTitle>
              </CardHeader>
              <CardContent>
                <span className={cn("flex items-center gap-1 text-xs", up ? "text-primary" : "text-destructive")}>
                  {up ? <TrendingUp className="size-3.5" /> : <TrendingDown className="size-3.5" />}
                  {k.hint} {up ? "+" : ""}
                  {k.delta}%
                </span>
              </CardContent>
            </Card>
          )
        })}
      </div>

      <div className="grid gap-4 lg:grid-cols-5">
        {/* 双折线对比 */}
        <Card className="lg:col-span-3">
          <CardHeader className="pb-2">
            <CardTitle className="text-base">活跃趋势 · 本期 vs 上期</CardTitle>
            <CardDescription>近 14 日活跃用户对比(演示数据)</CardDescription>
          </CardHeader>
          <CardContent className="h-72">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={compare} margin={{ top: 8, right: 8, left: -12, bottom: 0 }}>
                <defs>
                  <linearGradient id="athenaA" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="var(--chart-1)" stopOpacity={0.3} />
                    <stop offset="100%" stopColor="var(--chart-1)" stopOpacity={0.02} />
                  </linearGradient>
                  <linearGradient id="athenaB" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="var(--chart-4)" stopOpacity={0.22} />
                    <stop offset="100%" stopColor="var(--chart-4)" stopOpacity={0.02} />
                  </linearGradient>
                </defs>
                <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" vertical={false} />
                <XAxis dataKey="date" tick={{ fontSize: 11, fill: "var(--muted-foreground)" }} tickLine={false} axisLine={false} />
                <YAxis tick={{ fontSize: 11, fill: "var(--muted-foreground)" }} tickLine={false} axisLine={false} width={48} />
                <ReTooltip content={<Tip />} cursor={{ stroke: "var(--border)" }} />
                <Legend wrapperStyle={{ fontSize: 12 }} />
                <Area type="monotone" dataKey="本期" stroke="var(--chart-1)" strokeWidth={2} fill="url(#athenaA)" />
                <Area type="monotone" dataKey="上期" stroke="var(--chart-4)" strokeWidth={2} fill="url(#athenaB)" strokeDasharray="4 4" />
              </AreaChart>
            </ResponsiveContainer>
          </CardContent>
        </Card>

        {/* 渠道分布 */}
        <Card className="lg:col-span-2">
          <CardHeader className="pb-2">
            <CardTitle className="text-base">流量渠道分布</CardTitle>
            <CardDescription>近 7 日各渠道来源占比(演示数据)</CardDescription>
          </CardHeader>
          <CardContent className="h-72">
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <Pie data={channels} dataKey="value" nameKey="name" innerRadius={55} outerRadius={88} paddingAngle={3} strokeWidth={0}>
                  {channels.map((_, i) => (
                    <Cell key={i} fill={PIE_COLORS[i % PIE_COLORS.length]} />
                  ))}
                </Pie>
                <ReTooltip content={<Tip />} />
                <Legend wrapperStyle={{ fontSize: 12 }} />
              </PieChart>
            </ResponsiveContainer>
          </CardContent>
        </Card>
      </div>

      {/* 渠道明细 */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">渠道明细</CardTitle>
          <CardDescription>各渠道访问量与占比</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {channels.map((c, i) => {
            const total = channels.reduce((s, x) => s + x.value, 0)
            const pct = (c.value / total) * 100
            return (
              <div key={c.name} className="flex items-center gap-3">
                <span className="w-20 shrink-0 text-sm">{c.name}</span>
                <div className="h-2 flex-1 overflow-hidden rounded-full bg-muted">
                  <div
                    className="h-full rounded-full transition-all"
                    style={{ width: `${pct}%`, background: PIE_COLORS[i % PIE_COLORS.length] }}
                  />
                </div>
                <span className="w-24 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
                  {formatNumber(c.value)}({pct.toFixed(1)}%)
                </span>
              </div>
            )
          })}
        </CardContent>
      </Card>
    </div>
  )
}
