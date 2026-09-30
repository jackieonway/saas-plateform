import * as React from "react"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { formatDateTime } from "@/lib/format"

/**
 * 租户/组织「过期时间」共用展示工具 (后端契约 18-a):
 * expiresAt 为 ISO 字符串, null = 永不过期; 到期后该租户/组织下用户无法登录。
 * 旧后端未返回该字段时按 undefined 处理(等同永久), 不崩溃。
 */

/** 到期状态分类: permanent=永久/无字段, expired=已过期, soon=7 天内到期, later=更远 */
export type ExpiryState = "permanent" | "expired" | "soon" | "later"

/** 距到期阈值: 7 天内显示琥珀色「N 天后到期」 */
const SOON_WINDOW_MS = 7 * 24 * 60 * 60 * 1000

/**
 * ISO 字符串 → datetime-local 输入框值(本地时区, 格式 YYYY-MM-DDTHH:mm):
 * 编辑回填时不能直接 ISO.slice(0,16) —— 那是 UTC 字面值, 非零时区浏览器
 * 每次编辑再保存都会偏移时区数; 此处经 Date 按浏览器本地时区格式化,
 * 与提交端 new Date(v).toISOString() 的本地时区解释互为逆变换。
 */
export function isoToLocalInput(expiresAt: string | null | undefined): string {
  if (!expiresAt) return ""
  const d = new Date(expiresAt)
  if (Number.isNaN(d.getTime())) return ""
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
}

export function expiryState(expiresAt: string | null | undefined, now: number = Date.now()): ExpiryState {
  if (!expiresAt) return "permanent"
  const t = new Date(expiresAt).getTime()
  if (Number.isNaN(t)) return "permanent"
  if (t <= now) return "expired"
  if (t - now <= SOON_WINDOW_MS) return "soon"
  return "later"
}

/** 距到期整天数(向上取整, 过期返回 0; 永久/非法返回 null) */
export function daysUntilExpiry(expiresAt: string | null | undefined, now: number = Date.now()): number | null {
  if (!expiresAt) return null
  const t = new Date(expiresAt).getTime()
  if (Number.isNaN(t)) return null
  if (t <= now) return 0
  return Math.ceil((t - now) / (24 * 60 * 60 * 1000))
}

/**
 * 过期徽章(租户/组织两页共用):
 * - 已过期 → 红「已过期」(Tooltip 完整到期时间)
 * - 7 天内到期 → 琥珀「N 天后到期」(Tooltip 完整到期时间)
 * - 更远 / 永久 → 渲染 null(不显示)
 */
export function expiryBadge(expiresAt: string | null | undefined): React.ReactNode {
  const state = expiryState(expiresAt)
  if (state !== "expired" && state !== "soon") return null
  const days = daysUntilExpiry(expiresAt) ?? 0
  const label = state === "expired" ? "已过期" : days <= 1 ? "1 天后到期" : `${days} 天后到期`
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          tabIndex={0}
          role="status"
          className={
            state === "expired"
              ? "inline-flex shrink-0 items-center rounded-full border border-destructive/30 bg-destructive/10 px-1.5 py-px text-[10px] font-medium text-destructive"
              : "inline-flex shrink-0 items-center rounded-full border border-amber-500/30 bg-amber-500/10 px-1.5 py-px text-[10px] font-medium text-amber-600 dark:text-amber-400"
          }
        >
          {label}
        </span>
      </TooltipTrigger>
      <TooltipContent>
        {state === "expired" ? "已于 " : "将于 "}
        {formatDateTime(expiresAt)} 到期
      </TooltipContent>
    </Tooltip>
  )
}

/**
 * 表格「有效期」整格渲染(租户列表列 / 组织详情行共用):
 * - 永久(null/无字段) → 灰「—」(原生 title=永不过期)
 * - 已过期/7 天内 → 徽章(见 expiryBadge)
 * - 更远 → formatDateTime 灰字(悬停 title 同值)
 */
export function ExpiryCell({ expiresAt }: { expiresAt: string | null | undefined }) {
  const state = expiryState(expiresAt)
  if (state === "permanent") {
    return (
      <span className="text-xs text-muted-foreground/60" title="永不过期">
        —
      </span>
    )
  }
  if (state === "expired" || state === "soon") {
    return <span className="inline-flex items-center">{expiryBadge(expiresAt)}</span>
  }
  return (
    <span className="whitespace-nowrap text-xs text-muted-foreground" title={formatDateTime(expiresAt)}>
      {formatDateTime(expiresAt)}
    </span>
  )
}
