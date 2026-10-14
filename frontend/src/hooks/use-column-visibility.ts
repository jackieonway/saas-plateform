import * as React from "react"

/** 列定义:key 与表格渲染一一对应 */
export interface ColumnDef {
  key: string
  label: string
  /** 常显列(不可隐藏), 菜单中禁用并带锁提示 */
  required?: boolean
}

const STORAGE_PREFIX = "platform-cols-"
/** 同窗口内自定义广播事件(storage 事件仅跨标签页触发) */
const CHANGE_EVENT_PREFIX = "platform-cols-change-"

function storageKey(table: string): string {
  return STORAGE_PREFIX + table
}

function readVisible(table: string, columns: ColumnDef[]): Set<string> {
  const allKeys = columns.map((c) => c.key)
  const requiredKeys = new Set(columns.filter((c) => c.required).map((c) => c.key))
  try {
    const raw = localStorage.getItem(storageKey(table))
    if (!raw) return new Set(allKeys)
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return new Set(allKeys)
    const valid = new Set(allKeys)
    const kept = new Set(parsed.filter((k): k is string => typeof k === "string" && valid.has(k)))
    // required 列强制保留(防手工篡改 localStorage 后表格缺列)
    for (const k of requiredKeys) kept.add(k)
    return kept
  } catch {
    return new Set(allKeys)
  }
}

function writeVisible(table: string, visible: Iterable<string>) {
  try {
    localStorage.setItem(storageKey(table), JSON.stringify([...visible]))
  } catch {
    /* ignore */
  }
  window.dispatchEvent(new Event(CHANGE_EVENT_PREFIX + table))
}

/**
 * 表格列显隐(按表持久化):
 * - localStorage key `platform-cols-${table}` 存 visible 数组 JSON, 无存储时默认全显;
 * - required 列永不可隐藏(读取时强制补齐);
 * - 本窗口自定义事件 + 跨标签页 storage 事件实时同步;
 * - 切换即时生效,无闪烁。
 */
export function useColumnVisibility(table: string, columns: ColumnDef[]): {
  /** 当前可见列 key 集合 */
  visibleSet: Set<string>
  /** 单列是否可见 */
  isVisible: (key: string) => boolean
  /** 切换某列(对 required 列为 no-op) */
  toggle: (key: string) => void
  /** 直接设置可见列集合(持久化), 供下拉组件 onChange 透传 */
  setVisible: (next: Iterable<string>) => void
  /** 恢复全部列可见 */
  reset: () => void
} {
  const [visibleSet, setVisibleState] = React.useState<Set<string>>(() => readVisible(table, columns))

  React.useEffect(() => {
    const sync = () => setVisibleState(readVisible(table, columns))
    window.addEventListener(CHANGE_EVENT_PREFIX + table, sync)
    window.addEventListener("storage", sync)
    return () => {
      window.removeEventListener(CHANGE_EVENT_PREFIX + table, sync)
      window.removeEventListener("storage", sync)
    }
  }, [table, columns])

  const setVisible = React.useCallback(
    (next: Iterable<string>) => {
      const set = new Set(next)
      // required 列兜底
      for (const c of columns) if (c.required) set.add(c.key)
      writeVisible(table, set)
      setVisibleState(set)
    },
    [table, columns]
  )

  const toggle = React.useCallback(
    (key: string) => {
      const isRequired = columns.some((c) => c.key === key && c.required)
      if (isRequired) return
      const next = new Set(visibleSet)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      writeVisible(table, next)
      setVisibleState(next)
    },
    [table, columns, visibleSet]
  )

  const reset = React.useCallback(() => {
    const all = new Set(columns.map((c) => c.key))
    writeVisible(table, all)
    setVisibleState(all)
  }, [table, columns])

  const isVisible = React.useCallback((key: string) => visibleSet.has(key), [visibleSet])

  return { visibleSet, isVisible, toggle, setVisible, reset }
}
