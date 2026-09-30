import * as React from "react"
import { Check, Rows3, Rows4 } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useTableDensity, type TableDensity } from "@/hooks/use-table-density"
import { cn } from "@/lib/utils"

const OPTIONS: { value: TableDensity; label: string; icon: typeof Rows3; hint: string }[] = [
  { value: "comfortable", label: "舒适", icon: Rows4, hint: "标准行高" },
  { value: "compact", label: "紧凑", icon: Rows3, hint: "缩小行高,一屏更多" },
]

/**
 * 表格密度切换(全局):DropdownMenu 两选项 + 图标按钮。
 * 44px 触达区(size-11),放各列表页工具栏右侧;切换即时生效并持久化。
 */
export function TableDensityToggle({ className }: { className?: string }) {
  const { density, setDensity } = useTableDensity()

  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              variant="outline"
              size="icon"
              className={cn("size-11 text-muted-foreground", className)}
              aria-label="切换表格密度"
            >
              {density === "compact" ? (
                <Rows3 className="size-4" />
              ) : (
                <Rows4 className="size-4" />
              )}
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>表格密度:{density === "compact" ? "紧凑" : "舒适"}</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end" className="w-44">
        {OPTIONS.map((o) => (
          <DropdownMenuItem
            key={o.value}
            onClick={() => setDensity(o.value)}
            aria-checked={density === o.value}
            className="gap-2"
          >
            <o.icon className="size-4 text-muted-foreground" />
            <span className="flex-1">
              {o.label}
              <span className="ml-1.5 text-xs text-muted-foreground/70">{o.hint}</span>
            </span>
            {density === o.value && <Check className="size-4 text-primary" />}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
