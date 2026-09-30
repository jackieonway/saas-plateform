import * as React from "react"
import { ChevronsUpDown, Search, X } from "lucide-react"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { AppIcon, ICON_NAMES, ICON_REGISTRY } from "@/lib/icons"
import { cn } from "@/lib/utils"

/** 常用图标(选择器首位分组,覆盖系统各页签的典型图标选择) */
const COMMON_ICONS: string[] = [
  "Users", "User", "UserCog", "Shield", "ShieldCheck", "KeyRound", "Key",
  "Landmark", "Building2", "Network", "LayoutGrid", "LayoutDashboard", "Menu",
  "Settings", "Bell", "Database", "Server", "Globe", "Monitor", "Crown",
  "BarChart3", "LineChart", "PieChart", "FileText", "Inbox", "Send",
  "MessageSquare", "Workflow", "Activity", "Search", "Calendar", "Star",
]

/** 图标网格(8 列;方向键在网格内移动焦点) */
function IconGrid({
  names,
  value,
  onPick,
}: {
  names: string[]
  value: string
  onPick: (name: string) => void
}) {
  const gridRef = React.useRef<HTMLDivElement>(null)

  const onKeyDown = (e: React.KeyboardEvent) => {
    const keys = ["ArrowRight", "ArrowLeft", "ArrowUp", "ArrowDown"]
    if (!keys.includes(e.key)) return
    const buttons = Array.from(
      gridRef.current?.querySelectorAll<HTMLButtonElement>("[data-icon-item]") ?? []
    )
    if (buttons.length === 0) return
    const idx = buttons.indexOf(document.activeElement as HTMLButtonElement)
    let next = 0
    if (e.key === "ArrowRight") next = idx + 1
    else if (e.key === "ArrowLeft") next = idx - 1
    else if (e.key === "ArrowDown") next = idx + 8
    else next = idx - 8
    next = Math.max(0, Math.min(buttons.length - 1, next))
    buttons[next]?.focus()
    e.preventDefault()
  }

  if (names.length === 0) {
    return <p className="col-span-full py-4 text-center text-xs text-muted-foreground">—</p>
  }

  return (
    <div ref={gridRef} className="grid grid-cols-8 gap-1" onKeyDown={onKeyDown}>
      {names.map((name) => {
        const active = name === value
        return (
          <button
            key={name}
            data-icon-item
            type="button"
            title={name}
            aria-label={`选择图标 ${name}`}
            aria-pressed={active}
            onClick={() => onPick(name)}
            className={cn(
              "flex size-8 items-center justify-center rounded-md text-foreground/70 outline-none transition-colors",
              "hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
              active && "bg-primary/15 text-primary ring-1 ring-primary/40 hover:bg-primary/15 hover:text-primary"
            )}
          >
            <AppIcon name={name} className="size-4" />
          </button>
        )
      })}
    </div>
  )
}

/** 图标库选择器:搜索 + 常用/全部分组网格 + 实时预览(保持原 IconInput 契约) */
export function IconInput({
  value,
  onChange,
  placeholder = "从图标库选择",
  id,
  className,
}: {
  value: string
  onChange: (v: string) => void
  placeholder?: string
  id?: string
  className?: string
}) {
  const [open, setOpen] = React.useState(false)
  const [query, setQuery] = React.useState("")

  const needle = query.trim().toLowerCase()
  const searching = needle.length > 0
  const filtered = React.useMemo(() => {
    if (!needle) return ICON_NAMES
    return ICON_NAMES.filter((n) => n.toLowerCase().includes(needle))
  }, [needle])

  // 打开时重置搜索,聚焦即所见
  React.useEffect(() => {
    if (open) setQuery("")
  }, [open])

  const pick = (name: string) => {
    onChange(name)
    setOpen(false)
  }
  const clear = () => {
    onChange("")
    setOpen(false)
  }

  const commonIcons = React.useMemo(
    () => COMMON_ICONS.filter((n) => ICON_REGISTRY[n]),
    []
  )

  const sectionLabel = "px-1 pb-1 pt-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground first:pt-1"

  return (
    <div className={cn("relative", className)}>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            id={id}
            type="button"
            variant="outline"
            role="combobox"
            aria-expanded={open}
            className="w-full justify-between font-normal"
          >
            <span className="flex min-w-0 items-center gap-2">
              {value ? (
                <>
                  <AppIcon name={value} className="size-4 shrink-0 text-primary" />
                  <span className="truncate font-mono text-[13px]">{value}</span>
                </>
              ) : (
                <span className="text-muted-foreground">{placeholder}</span>
              )}
            </span>
            <ChevronsUpDown className="size-4 shrink-0 opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="w-[var(--radix-popover-trigger-width)] min-w-72 p-0"
        >
          {/* 搜索栏 */}
          <div className="flex items-center gap-2 border-b px-3">
            <Search className="size-4 shrink-0 text-muted-foreground" />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索图标名,如 users…"
              className="h-9 border-0 px-0 shadow-none focus-visible:ring-0"
              autoComplete="off"
              aria-label="搜索图标"
            />
            {searching && (
              <button
                type="button"
                onClick={() => setQuery("")}
                aria-label="清空搜索"
                className="flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
              >
                <X className="size-3.5" />
              </button>
            )}
          </div>

          {/* 图标网格 */}
          <ScrollArea className="h-72">
            <div className="p-2">
              {filtered.length === 0 ? (
                <p className="py-10 text-center text-xs text-muted-foreground">
                  没有匹配「{query.trim()}」的图标
                </p>
              ) : searching ? (
                <IconGrid names={filtered} value={value} onPick={pick} />
              ) : (
                <>
                  <p className={sectionLabel}>常用</p>
                  <IconGrid names={commonIcons} value={value} onPick={pick} />
                  <p className={sectionLabel}>全部图标</p>
                  <IconGrid names={filtered} value={value} onPick={pick} />
                </>
              )}
            </div>
          </ScrollArea>

          {/* 底部状态栏 */}
          <div className="flex items-center justify-between gap-2 border-t px-3 py-2">
            <p className="min-w-0 truncate text-xs text-muted-foreground">
              {value ? (
                <>
                  当前 <span className="font-mono text-foreground">{value}</span>
                </>
              ) : (
                "未选择图标"
              )}
              <span className="ml-1.5 tabular-nums">
                {searching ? `${filtered.length}/${ICON_NAMES.length}` : ICON_NAMES.length} 个
              </span>
            </p>
            {value && (
              <Button
                variant="ghost"
                size="sm"
                className="h-7 shrink-0 gap-1 px-2 text-xs"
                onClick={clear}
              >
                <X className="size-3" />
                清除
              </Button>
            )}
          </div>
        </PopoverContent>
      </Popover>
    </div>
  )
}
