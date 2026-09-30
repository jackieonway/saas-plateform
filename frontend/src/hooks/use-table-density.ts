import * as React from "react"

/** 表格密度:comfortable(舒适,默认) / compact(紧凑) */
export type TableDensity = "compact" | "comfortable"

const STORAGE_KEY = "dante-table-density"
/** 同窗口内自定义广播事件(storage 事件仅跨标签页触发) */
const CHANGE_EVENT = "dante-table-density-change"

function readDensity(): TableDensity {
  try {
    return localStorage.getItem(STORAGE_KEY) === "compact" ? "compact" : "comfortable"
  } catch {
    return "comfortable"
  }
}

function writeDensity(d: TableDensity) {
  try {
    localStorage.setItem(STORAGE_KEY, d)
  } catch {
    /* ignore */
  }
  window.dispatchEvent(new Event(CHANGE_EVENT))
}

/**
 * 全局表格密度(列表页共用):
 * - localStorage 持久化,刷新后保持;
 * - 本窗口自定义事件 + 跨标签页 storage 事件实时同步;
 * - 切换即时生效,无需刷新页面。
 */
export function useTableDensity(): {
  density: TableDensity
  setDensity: (d: TableDensity) => void
  toggle: () => void
} {
  const [density, setDensityState] = React.useState<TableDensity>(readDensity)

  React.useEffect(() => {
    const sync = () => setDensityState(readDensity())
    window.addEventListener(CHANGE_EVENT, sync)
    window.addEventListener("storage", sync)
    return () => {
      window.removeEventListener(CHANGE_EVENT, sync)
      window.removeEventListener("storage", sync)
    }
  }, [])

  const setDensity = React.useCallback((d: TableDensity) => {
    writeDensity(d)
    setDensityState(d)
  }, [])

  const toggle = React.useCallback(() => {
    const next: TableDensity = readDensity() === "compact" ? "comfortable" : "compact"
    writeDensity(next)
    setDensityState(next)
  }, [])

  return { density, setDensity, toggle }
}

/**
 * 密度 → TableBody 附加 class(侵入最小的实现方式):
 * 给 <TableBody> 传后代选择器 class,一次性作用于全部 TableRow/TableCell,
 * 不改 shadcn table 组件本身。comfortable 与现状完全一致(不附加 class)。
 */
export const DENSITY_BODY_CLASS: Record<TableDensity, string | undefined> = {
  comfortable: undefined,
  compact: "[&_td]:py-1 [&_td]:text-[13px] [&_th]:h-8",
}
