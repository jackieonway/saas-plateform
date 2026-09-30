import * as React from "react"
import { ArrowRight, ChevronLeft, ChevronRight } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Pagination,
  PaginationContent,
  PaginationEllipsis,
  PaginationItem,
  PaginationLink,
} from "@/components/ui/pagination"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

function pageRange(current: number, total: number): (number | "ellipsis-l" | "ellipsis-r")[] {
  if (total <= 7) return Array.from({ length: total }, (_, i) => i + 1)
  const pages: (number | "ellipsis-l" | "ellipsis-r")[] = [1]
  if (current > 3) pages.push("ellipsis-l")
  for (let p = Math.max(2, current - 1); p <= Math.min(total - 1, current + 1); p++) pages.push(p)
  if (current < total - 2) pages.push("ellipsis-r")
  pages.push(total)
  return pages
}

export function DataPagination({
  page,
  size,
  total,
  onPageChange,
  onSizeChange,
  sizeOptions = [10, 20, 50],
  className,
}: {
  page: number
  size: number
  total: number
  onPageChange: (page: number) => void
  onSizeChange?: (size: number) => void
  sizeOptions?: number[]
  className?: string
}) {
  const totalPages = Math.max(1, Math.ceil(total / size))
  const current = Math.min(page, totalPages)
  const canPrev = current > 1
  const canNext = current < totalPages

  /* 跳页输入:本地字符串 state, 提交时 parseInt + clamp 到 [1, totalPages] */
  const [jumpInput, setJumpInput] = React.useState(String(current))
  React.useEffect(() => {
    /* 页码变化(翻页/跳页/改每页) → 显示值同步回当前页码 */
    setJumpInput(String(current))
  }, [current])

  const submitJump = () => {
    const parsed = Number.parseInt(jumpInput.trim(), 10)
    if (Number.isNaN(parsed)) {
      setJumpInput(String(current))
      return
    }
    const clamped = Math.min(Math.max(parsed, 1), totalPages)
    setJumpInput(String(clamped))
    if (clamped !== current) onPageChange(clamped)
  }

  return (
    <div className={cn("flex flex-wrap flex-col gap-3 sm:flex-row sm:items-center sm:justify-between", className)}>
      {/* 左:信息组(总数 + 每页条数) */}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span>
          共 <span className="font-medium text-foreground tabular-nums">{total}</span> 条
        </span>
        {onSizeChange && (
          <span className="flex items-center gap-1.5">
            <span>每页</span>
            <Select
              value={String(size)}
              onValueChange={(v) => onSizeChange(Number(v))}
            >
              <SelectTrigger size="sm" className="h-7 w-[4.5rem] text-xs" aria-label="每页条数">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {sizeOptions.map((s) => (
                  <SelectItem key={s} value={String(s)}>
                    {s} 条
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </span>
        )}
      </div>

      {/* 右:页码组 + 跳页 */}
      <div className="flex max-w-full flex-wrap items-center justify-center gap-2">
        {/* 页码区:窄屏横向滚动防挤压变形 */}
        <div className="max-w-full overflow-x-auto scroll-area">
          <Pagination className="mx-0 w-auto justify-center">
            <PaginationContent>
              <PaginationItem>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="outline"
                      size="icon"
                      className="size-8"
                      aria-label="上一页"
                      disabled={!canPrev}
                      onClick={() => canPrev && onPageChange(current - 1)}
                    >
                      <ChevronLeft className="size-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>上一页 ({current}/{totalPages})</TooltipContent>
                </Tooltip>
              </PaginationItem>
              {pageRange(current, totalPages).map((p, idx) =>
                typeof p === "number" ? (
                  <PaginationItem key={p}>
                    <PaginationLink
                      href="#"
                      isActive={p === current}
                      onClick={(e) => {
                        e.preventDefault()
                        if (p !== current) onPageChange(p)
                      }}
                    >
                      {p}
                    </PaginationLink>
                  </PaginationItem>
                ) : (
                  <PaginationItem key={`${p}-${idx}`}>
                    <PaginationEllipsis />
                  </PaginationItem>
                )
              )}
              <PaginationItem>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="outline"
                      size="icon"
                      className="size-8"
                      aria-label="下一页"
                      disabled={!canNext}
                      onClick={() => canNext && onPageChange(current + 1)}
                    >
                      <ChevronRight className="size-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>下一页 ({current}/{totalPages})</TooltipContent>
                </Tooltip>
              </PaginationItem>
            </PaginationContent>
          </Pagination>
        </div>

        {/* 跳页:单页时无意义,隐藏 */}
        {totalPages > 1 && (
          <>
            <span className="hidden h-5 w-px bg-border sm:block" aria-hidden />
            <span className="flex items-center gap-1 text-xs text-muted-foreground">
              <span>第</span>
              <Input
                type="text"
                inputMode="numeric"
                className="h-8 w-12 px-1 text-center tabular-nums"
                aria-label={`跳转到指定页 (共 ${totalPages} 页)`}
                value={jumpInput}
                onChange={(e) => setJumpInput(e.target.value.replace(/[^\d]/g, ""))}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault()
                    submitJump()
                  }
                }}
                onBlur={() => setJumpInput(String(current))}
              />
              <span>
                / <span className="tabular-nums">{totalPages}</span> 页
              </span>
              {/* mousedown 阻止默认, 防止点击按钮先触发 input 失焦回填导致跳页失效 */}
              <Button
                variant="outline"
                size="sm"
                className="h-8 gap-1 px-2.5"
                aria-label="跳转"
                onMouseDown={(e) => e.preventDefault()}
                onClick={submitJump}
              >
                跳转
                <ArrowRight className="size-3.5" />
              </Button>
            </span>
          </>
        )}
      </div>
    </div>
  )
}
