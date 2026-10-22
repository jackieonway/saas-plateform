import * as React from "react"
import { Building2, ChevronDown, ChevronRight, Layers, Search } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { ScrollArea } from "@/components/ui/scroll-area"
import { EmptyState } from "@/components/states"
import { cn } from "@/lib/utils"

export interface PickerNode {
  id: string
  name: string
  code?: string
  status?: string
  children?: PickerNode[]
}

/** 按名称过滤树(命中节点的父链保留) */
export function filterTree<T extends PickerNode>(nodes: T[], keyword: string): T[] {
  if (!keyword.trim()) return nodes
  const kw = keyword.trim().toLowerCase()
  const walk = (list: T[]): T[] => {
    const out: T[] = []
    for (const n of list) {
      const hit = n.name.toLowerCase().includes(kw) || (n.code ?? "").toLowerCase().includes(kw)
      const subs = n.children ? walk(n.children as T[]) : []
      if (hit || subs.length > 0) {
        out.push({ ...n, children: subs.length > 0 ? subs : hit ? (n.children ?? []) : subs })
      }
    }
    return out
  }
  return walk(nodes)
}

export function flattenPicker<T extends PickerNode>(nodes: T[], out: T[] = []): T[] {
  for (const n of nodes) {
    out.push(n)
    if (n.children) flattenPicker(n.children as T[], out)
  }
  return out
}

/** 树形选择器(Popover) */
export function EntityTreePicker({
  nodes,
  value,
  onSelect,
  placeholder = "选择组织",
  className,
  allowClear = true,
  emptyText = "暂无数据",
  icon,
  disabled,
  ariaLabel,
}: {
  nodes: PickerNode[]
  value?: string | null
  onSelect: (id: string | null) => void
  placeholder?: string
  className?: string
  allowClear?: boolean
  emptyText?: string
  icon?: React.ReactNode
  disabled?: boolean
  ariaLabel?: string
}) {
  const [open, setOpen] = React.useState(false)
  const [keyword, setKeyword] = React.useState("")

  const findName = (list: PickerNode[], id: string | null): string | null => {
    if (!id) return null
    for (const n of list) {
      if (n.id === id) return n.name
      const sub = n.children ? findName(n.children, id) : null
      if (sub) return sub
    }
    return null
  }
  const selectedName = findName(nodes, value ?? null)
  const shown = React.useMemo(() => filterTree(nodes, keyword), [nodes, keyword])

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          role="combobox"
          aria-label={ariaLabel ?? placeholder}
          aria-expanded={open}
          disabled={disabled}
          className={cn("h-9 w-full justify-between font-normal", !selectedName && "text-muted-foreground", className)}
        >
          <span className="flex min-w-0 items-center gap-2">
            {icon ?? <Building2 className="size-4 shrink-0 text-muted-foreground" />}
            <span className="truncate">{selectedName ?? placeholder}</span>
          </span>
          <ChevronDown className="size-4 shrink-0 opacity-50" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[min(20rem,calc(100vw-2rem))] p-0">
        <div className="flex items-center gap-2 border-b px-3 py-2">
          <Search className="size-4 shrink-0 text-muted-foreground" />
          <Input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="搜索名称 / 编码"
            className="h-7 border-0 shadow-none focus-visible:ring-0"
          />
        </div>
        <ScrollArea className="h-64">
          <div className="p-1.5">
            {allowClear && value && (
              <button
                type="button"
                className="w-full rounded-md px-2 py-1.5 text-left text-sm text-muted-foreground hover:bg-accent"
                onClick={() => {
                  onSelect(null)
                  setOpen(false)
                }}
              >
                清除选择
              </button>
            )}
            {shown.length === 0 ? (
              <EmptyState title={emptyText} description="换个关键词试试" className="py-8" icon={<Layers className="size-5" />} />
            ) : (
              shown.map((n) => (
                <PickerRow
                  key={n.id}
                  node={n}
                  depth={0}
                  selectedId={value ?? null}
                  onSelect={(id) => {
                    onSelect(id)
                    setOpen(false)
                  }}
                />
              ))
            )}
          </div>
        </ScrollArea>
      </PopoverContent>
    </Popover>
  )
}

function PickerRow({
  node,
  depth,
  selectedId,
  onSelect,
}: {
  node: PickerNode
  depth: number
  selectedId: string | null
  onSelect: (id: string) => void
}) {
  const [open, setOpen] = React.useState(depth < 2)
  const hasChildren = !!node.children && node.children.length > 0
  const active = selectedId === node.id
  return (
    <div>
      <button
        type="button"
        className={cn(
          "flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
          active ? "bg-primary/10 font-medium text-primary" : "hover:bg-accent",
          depth > 0 && "ml-3"
        )}
        style={{ paddingLeft: depth * 14 + 8 }}
        onClick={() => onSelect(node.id)}
      >
        {hasChildren ? (
          <span
            role="button"
            tabIndex={-1}
            aria-label={open ? "折叠" : "展开"}
            className="flex size-4 shrink-0 items-center justify-center text-muted-foreground"
            onClick={(e) => {
              e.stopPropagation()
              setOpen((o) => !o)
            }}
          >
            {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
          </span>
        ) : (
          <span className="w-4 shrink-0" />
        )}
        <span className="truncate">{node.name}</span>
        {node.code && <span className="ml-auto shrink-0 font-mono text-xs text-muted-foreground/60">{node.code}</span>}
      </button>
      {hasChildren && open && (
        <div>
          {node.children!.map((c) => (
            <PickerRow key={c.id} node={c} depth={depth + 1} selectedId={selectedId} onSelect={onSelect} />
          ))}
        </div>
      )}
    </div>
  )
}
