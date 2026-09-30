import { format, formatDistanceToNowStrict, isValid, parseISO } from "date-fns"
import { zhCN } from "date-fns/locale"

function toDate(value: string | Date | null | undefined): Date | null {
  if (!value) return null
  const d = typeof value === "string" ? parseISO(value) : value
  return isValid(d) ? d : null
}

/** 2026-01-02 15:04 */
export function formatDateTime(value: string | Date | null | undefined): string {
  const d = toDate(value)
  if (!d) return "—"
  return format(d, "yyyy-MM-dd HH:mm")
}

/** 2026-01-02 */
export function formatDate(value: string | Date | null | undefined): string {
  const d = toDate(value)
  if (!d) return "—"
  return format(d, "yyyy-MM-dd")
}

/** 相对时间:3 分钟前 / 5 小时前 / 2 天前 */
export function formatRelative(value: string | Date | null | undefined): string {
  const d = toDate(value)
  if (!d) return "—"
  try {
    return formatDistanceToNowStrict(d, { addSuffix: true, locale: zhCN })
  } catch {
    return "—"
  }
}

export function formatNumber(n: number | null | undefined): string {
  if (n === null || n === undefined || Number.isNaN(n)) return "—"
  return new Intl.NumberFormat("zh-CN").format(n)
}

/** 字节大小人性化: B → KB → MB → GB → TB (保留 1 位小数, ≥100 取整) */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || Number.isNaN(bytes) || bytes < 0) return "—"
  if (bytes < 1024) return `${bytes} B`
  const units = ["KB", "MB", "GB", "TB"]
  let v = bytes
  let i = -1
  do {
    v /= 1024
    i++
  } while (v >= 1024 && i < units.length - 1)
  return `${v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[i]}`
}

/** 运行时长人性化: 秒数 → 「3天 4小时 5分 6秒」(高位为 0 时省略, 全 0 显示「0秒」) */
export function durationHuman(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || Number.isNaN(seconds) || seconds < 0) return "—"
  const s = Math.floor(seconds)
  const parts: string[] = []
  const push = (n: number, unit: string) => {
    if (n > 0) parts.push(`${n}${unit}`)
  }
  push(Math.floor(s / 86_400), "天")
  push(Math.floor((s % 86_400) / 3_600), "小时")
  push(Math.floor((s % 3_600) / 60), "分")
  if (s % 60 > 0 || parts.length === 0) parts.push(`${s % 60}秒`)
  return parts.join(" ")
}

/** 生成强随机密码:大小写字母+数字+符号,12 位 */
export function generatePassword(length = 12): string {
  const upper = "ABCDEFGHJKLMNPQRSTUVWXYZ"
  const lower = "abcdefghijkmnopqrstuvwxyz"
  const digits = "23456789"
  const symbols = "!@#$%^&*"
  const all = upper + lower + digits + symbols
  const pick = (s: string) => s[crypto.getRandomValues(new Uint32Array(1))[0] % s.length]
  let pwd = pick(upper) + pick(lower) + pick(digits) + pick(symbols)
  while (pwd.length < length) pwd += pick(all)
  // 洗牌
  const arr = pwd.split("")
  for (let i = arr.length - 1; i > 0; i--) {
    const j = crypto.getRandomValues(new Uint32Array(1))[0] % (i + 1)
    ;[arr[i], arr[j]] = [arr[j], arr[i]]
  }
  return arr.join("")
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    try {
      const ta = document.createElement("textarea")
      ta.value = text
      ta.style.position = "fixed"
      ta.style.opacity = "0"
      document.body.appendChild(ta)
      ta.select()
      document.execCommand("copy")
      document.body.removeChild(ta)
      return true
    } catch {
      return false
    }
  }
}

/** 防抖 hook */
import * as React from "react"
export function useDebounced<T>(value: T, delay = 300): T {
  const [debounced, setDebounced] = React.useState(value)
  React.useEffect(() => {
    const t = setTimeout(() => setDebounced(value), delay)
    return () => clearTimeout(t)
  }, [value, delay])
  return debounced
}

/** scope / 动作等枚举的中文映射 */
export const SCOPE_LABELS: Record<string, { label: string; icon: string; desc: string }> = {
  profile: { label: "基础资料", icon: "User", desc: "用户名、姓名等基础信息" },
  email: { label: "邮箱", icon: "Mail", desc: "电子邮箱地址" },
  phone: { label: "手机号", icon: "Phone", desc: "手机号码" },
  roles: { label: "角色信息", icon: "BadgeCheck", desc: "所属角色列表" },
  read: { label: "只读数据", icon: "Eye", desc: "只读访问您的数据" },
  write: { label: "写入数据", icon: "Edit", desc: "以您的名义写入数据" },
  offline_access: { label: "离线访问", icon: "RefreshCw", desc: "在您离线时持续访问" },
}

export const GRANT_TYPE_LABELS: Record<string, string> = {
  authorization_code: "授权码模式",
  refresh_token: "刷新令牌",
  client_credentials: "客户端凭证",
  password: "资源密码模式",
  "urn:ietf:params:oauth:grant-type:device_code": "设备码模式",
}

export const GENDER_LABELS: Record<string, string> = {
  MALE: "男",
  FEMALE: "女",
  UNKNOWN: "未知",
}

export const MENU_TYPE_META: Record<string, { label: string; className: string }> = {
  DIR: { label: "目录", className: "border-warning/40 bg-warning/10 text-warning" },
  MENU: { label: "菜单", className: "border-primary/30 bg-primary/10 text-primary" },
  BUTTON: { label: "按钮", className: "border-border bg-muted text-muted-foreground" },
}

export function greeting(): string {
  const h = new Date().getHours()
  if (h < 6) return "夜深了"
  if (h < 12) return "早安"
  if (h < 14) return "午安"
  if (h < 18) return "下午好"
  return "晚上好"
}
