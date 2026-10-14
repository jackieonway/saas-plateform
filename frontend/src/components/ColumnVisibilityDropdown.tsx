import * as React from "react"
import { Columns3, EyeOff, Lock, RotateCcw } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import type { ColumnDef } from "@/hooks/use-column-visibility"
import { cn } from "@/lib/utils"

/**
 * 表格列显隐下拉(与表格密度切换同族交互):
 * - 触发按钮: Columns3 图标 outline, size-11(44px 触达区), tooltip「显示/隐藏列」;
 * - 内容: CheckboxItem 列表(列名 + 当前状态; required 列禁用 + Locked 图标提示);
 * - 底部: 已隐藏计数 + 「重置」按钮(恢复全部列);
 * - 可见列由父级 useColumnVisibility 持有, 变更经 onChange 透传持久化。
 */
export function ColumnVisibilityDropdown({
  table,
  columns,
  visible,
  onChange,
  className,
}: {
  /** 存储 key(localStorage: platform-cols-${table}) */
  table: string
  columns: ColumnDef[]
  /** 当前可见列 key 数组 */
  visible: string[]
  /** 提交新的可见列集合 */
  onChange: (next: string[]) => void
  className?: string
}) {
  const visibleSet = React.useMemo(() => new Set(visible), [visible])
  const hiddenCount = columns.filter((c) => !visibleSet.has(c.key)).length

  const toggle = (key: string, required?: boolean) => {
    if (required) return
    const next = new Set(visibleSet)
    if (next.has(key)) next.delete(key)
    else next.add(key)
    onChange([...next])
  }

  const reset = () => onChange(columns.map((c) => c.key))

  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              variant="outline"
              size="icon"
              className={cn("size-11 text-muted-foreground", className)}
              aria-label="显示或隐藏列"
            >
              <Columns3 className="size-4" />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>显示 / 隐藏列</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end" className="w-52">
        <p className="px-2 py-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
          表格列
          <span className="ml-1.5 normal-case tracking-normal text-muted-foreground/70">
            {columns.length - hiddenCount}/{columns.length}
          </span>
        </p>
        <DropdownMenuSeparator className="my-1" />
        {columns.map((c) => {
          const checked = visibleSet.has(c.key)
          return (
            <DropdownMenuCheckboxItem
              key={`${table}-${c.key}`}
              checked={checked}
              disabled={c.required}
              onCheckedChange={() => toggle(c.key, c.required)}
              onSelect={(e) => e.preventDefault()}
              className="gap-2"
            >
              <span className="flex-1 truncate">{c.label}</span>
              {c.required ? (
                <Lock className="size-3 shrink-0 text-muted-foreground/50" aria-label="常显列" />
              ) : (
                <span
                  className={cn(
                    "shrink-0 text-[10px]",
                    checked ? "text-muted-foreground/60" : "text-muted-foreground/80"
                  )}
                >
                  {checked ? "显示中" : "已隐藏"}
                </span>
              )}
            </DropdownMenuCheckboxItem>
          )
        })}
        <DropdownMenuSeparator className="my-1" />
        <div className="flex items-center justify-between px-2 py-1">
          <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/70">
            {hiddenCount > 0 ? (
              <>
                <EyeOff className="size-3" />
                已隐藏 <span className="tabular-nums">{hiddenCount}</span> 列
              </>
            ) : (
              "全部列显示中"
            )}
          </span>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1 px-2 text-xs text-muted-foreground"
            onClick={reset}
            disabled={hiddenCount === 0}
          >
            <RotateCcw className="size-3" />
            重置
          </Button>
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
