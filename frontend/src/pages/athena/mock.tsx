import * as React from "react"

/** Athena 数据分析演示应用 mock 数据(useMemo 稳定) */

export interface Kpi {
  label: string
  value: string
  delta: number
  hint: string
}

export function useAthenaOverview() {
  return React.useMemo(() => {
    const kpis: Kpi[] = [
      { label: "今日活跃用户", value: "12,480", delta: 8.2, hint: "较昨日" },
      { label: "新增用户", value: "326", delta: -4.5, hint: "较昨日" },
      { label: "7 日留存率", value: "68.4%", delta: 2.1, hint: "较上周" },
      { label: "平均使用时长", value: "14m 36s", delta: 5.7, hint: "较上周" },
    ]
    const compare = Array.from({ length: 14 }).map((_, i) => {
      const d = new Date(Date.now() - (13 - i) * 86400_000)
      const mm = String(d.getMonth() + 1).padStart(2, "0")
      const dd = String(d.getDate()).padStart(2, "0")
      return {
        date: `${mm}-${dd}`,
        本期: 3200 + Math.round(Math.sin(i / 2.4) * 900 + i * 130),
        上期: 2800 + Math.round(Math.cos(i / 3.1) * 700 + i * 90),
      }
    })
    const channels = [
      { name: "自然流量", value: 4210 },
      { name: "搜索引擎", value: 3480 },
      { name: "社交媒体", value: 2760 },
      { name: "直接访问", value: 1950 },
      { name: "邮件营销", value: 860 },
      { name: "其他", value: 520 },
    ]
    return { kpis, compare, channels }
  }, [])
}

export function useAthenaSales(range: string) {
  return React.useMemo(() => {
    const factor = range === "近 30 天" ? 30 : range === "近 7 天" ? 7 : 90
    const byCategory = ["电子数码", "家居日用", "服饰鞋包", "美妆个护", "食品生鲜"].map((name, gi) => ({
      category: name,
      销售额: Math.round((850 + gi * 320) * factor * (0.8 + ((gi * 37) % 10) / 25)),
      订单量: Math.round((420 + gi * 130) * factor * (0.7 + ((gi * 53) % 10) / 20)),
    }))
    const details = Array.from({ length: 8 }).map((_, i) => ({
      id: `SO-${String(10250 + i)}`,
      customer: ["上海云徙科技", "杭州数澜信息", "北京灵犀智能", "深圳极光数据", "广州鲸图网络"][i % 5],
      amount: Math.round((12000 + i * 3571) * 100) / 100,
      count: 12 + ((i * 7) % 40),
      status: i % 4 === 0 ? "已完成" : i % 4 === 1 ? "进行中" : i % 4 === 2 ? "待支付" : "已完成",
      date: `2026-0${(i % 3) + 1}-${String(12 + i).padStart(2, "0")}`,
    }))
    const totalAmount = details.reduce((s, d) => s + d.amount, 0)
    const totalCount = details.reduce((s, d) => s + d.count, 0)
    return { byCategory, details, totalAmount, totalCount }
  }, [range])
}

export function useAthenaOperations() {
  return React.useMemo(() => {
    const funnel = [
      { stage: "访问首页", count: 86400, pct: 100 },
      { stage: "浏览商品", count: 52400, pct: 60.6 },
      { stage: "加入购物车", count: 21800, pct: 25.2 },
      { stage: "提交订单", count: 12600, pct: 14.6 },
      { stage: "完成支付", count: 9820, pct: 11.4 },
    ]
    const table = Array.from({ length: 6 }).map((_, i) => ({
      region: ["华东", "华北", "华南", "西南", "东北", "西北"][i],
      orders: 3200 - i * 410,
      gmv: Math.round((982000 - i * 121000) * 100) / 100,
      conversion: `${(6.4 - i * 0.7).toFixed(1)}%`,
      satisfaction: `${(92 - i * 3.2).toFixed(1)}`,
    }))
    return { funnel, table }
  }, [])
}

export interface DataSource {
  name: string
  type: "MySQL" | "PostgreSQL" | "Kafka" | "Redis"
  host: string
  status: "UP" | "PENDING" | "DOWN"
  latencyMs: number
  lastSyncAt: string
  tables: number
}

export function useAthenaSources() {
  return React.useMemo<DataSource[]>(() => {
    const mins = (m: number) => new Date(Date.now() - m * 60_000).toISOString()
    return [
      { name: "订单主库", type: "MySQL", host: "mysql-order.internal:3306", status: "UP", latencyMs: 12, lastSyncAt: mins(3), tables: 42 },
      { name: "用户行为分析", type: "PostgreSQL", host: "pg-analytics.internal:5432", status: "UP", latencyMs: 18, lastSyncAt: mins(1), tables: 28 },
      { name: "实时事件流", type: "Kafka", host: "kafka-01.internal:9092", status: "UP", latencyMs: 6, lastSyncAt: mins(0), tables: 12 },
      { name: "缓存热点", type: "Redis", host: "redis-cache.internal:6379", status: "PENDING", latencyMs: 3, lastSyncAt: mins(26), tables: 0 },
      { name: "商品库(只读副本)", type: "MySQL", host: "mysql-product-ro.internal:3306", status: "UP", latencyMs: 24, lastSyncAt: mins(9), tables: 36 },
      { name: "日志归档", type: "PostgreSQL", host: "pg-logs.internal:5432", status: "DOWN", latencyMs: 0, lastSyncAt: mins(184), tables: 64 },
    ]
  }, [])
}

export const SOURCE_TYPE_META: Record<DataSource["type"], { icon: string; className: string }> = {
  MySQL: { icon: "Database", className: "bg-chart-5/15 text-chart-5" },
  PostgreSQL: { icon: "Database", className: "bg-chart-4/15 text-chart-4" },
  Kafka: { icon: "Workflow", className: "bg-chart-2/15 text-chart-2" },
  Redis: { icon: "Zap", className: "bg-primary/15 text-primary" },
}

/** 演示应用徽章 */
export function DemoBadge() {
  return (
    <span className="inline-flex items-center gap-1 rounded-md border border-warning/40 bg-warning/10 px-2 py-0.5 text-xs font-medium text-warning">
      演示应用
    </span>
  )
}
