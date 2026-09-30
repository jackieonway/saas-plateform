import * as React from "react"

/** 最近访问记录(存 localStorage,个人工作台展示) */
export interface RecentVisit {
  path: string
  title: string
  /** 图标名(lib/icons 注册表) */
  icon: string
  /** 访问时间戳(ms) */
  at: number
  /** 访问次数(同 path 去重累计;旧数据无此字段时视为 1) */
  count: number
}

const STORAGE_KEY = "dante-recent-visits"
const MAX_VISITS = 8
/** 同窗口内自定义广播事件(storage 事件仅跨标签页触发) */
const CHANGE_EVENT = "dante-recent-visits-change"

function readVisits(): RecentVisit[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed
      .filter(
        (v): v is RecentVisit =>
          !!v &&
          typeof v.path === "string" &&
          typeof v.title === "string" &&
          typeof v.icon === "string"
      )
      .map((v) => ({
        ...v,
        at: typeof v.at === "number" ? v.at : Date.now(),
        // 旧数据无 count 时默认 1;非法值(非正数/非整数)一律归一到 1
        count: normalizeCount(v.count),
      }))
      .slice(0, MAX_VISITS)
  } catch {
    return []
  }
}

/** 访问次数归一: 正整数取原值, 其余(含旧数据缺省)回退 1 */
function normalizeCount(count: unknown): number {
  return typeof count === "number" && Number.isFinite(count) && count >= 1
    ? Math.floor(count)
    : 1
}

function writeVisits(visits: RecentVisit[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(visits.slice(0, MAX_VISITS)))
  } catch {
    /* ignore */
  }
  window.dispatchEvent(new Event(CHANGE_EVENT))
}

/**
 * 记录一次页面访问:按 path 去重(count 在旧值上 +1),最新在前,最多 8 条。
 */
export function pushRecentVisit(entry: { path: string; title: string; icon?: string }) {
  const prev = readVisits()
  const existing = prev.find((v) => v.path === entry.path)
  const rest = prev.filter((v) => v.path !== entry.path)
  const visit: RecentVisit = {
    path: entry.path,
    title: entry.title,
    icon: entry.icon || existing?.icon || "Page",
    at: Date.now(),
    count: (existing?.count ?? 0) + 1,
  }
  writeVisits([visit, ...rest].slice(0, MAX_VISITS))
}

export function clearRecentVisits() {
  writeVisits([])
}

/** 订阅最近访问列表(本窗口自定义事件 + 跨标签页 storage 事件) */
export function useRecentVisits(): RecentVisit[] {
  const [visits, setVisits] = React.useState<RecentVisit[]>(readVisits)
  React.useEffect(() => {
    const sync = () => setVisits(readVisits())
    window.addEventListener(CHANGE_EVENT, sync)
    window.addEventListener("storage", sync)
    return () => {
      window.removeEventListener(CHANGE_EVENT, sync)
      window.removeEventListener("storage", sync)
    }
  }, [])
  return visits
}
