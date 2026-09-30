import * as React from "react"
import { Building2, ChevronDown, ChevronRight, Network, Pencil, Plus, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { EmptyState } from "@/components/states"
import { cn } from "@/lib/utils"

export interface EditableNode {
  id: string
  name: string
  code?: string
  status?: string
  description?: string | null
  /** 过期时间(ISO, null/缺省=永不过期; 组织树行用于渲染过期/临期徽章, 部门树不传) */
  expiresAt?: string | null
  children: EditableNode[]
}

/** 通用树编辑器:节点 hover 显示 新增子项/编辑/删除 */
export function TreeEditor({
  nodes,
  onAdd,
  onEdit,
  onDelete,
  emptyTitle = "暂无数据",
  emptyDescription,
  addLabel = "新增根节点",
  onAddRoot,
  className,
}: {
  nodes: EditableNode[]
  onAdd: (parentId: string) => void
  onEdit: (node: EditableNode) => void
  onDelete: (node: EditableNode) => void
  emptyTitle?: string
  emptyDescription?: string
  addLabel?: string
  onAddRoot: () => void
  className?: string
}) {
  if (nodes.length === 0) {
    return (
      <EmptyState
        className="py-10"
        title={emptyTitle}
        description={emptyDescription}
        icon={<Network className="size-5" />}
        action={
          <Button size="sm" variant="outline" onClick={onAddRoot}>
            <Plus className="size-4" />
            {addLabel}
          </Button>
        }
      />
    )
  }
  return (
    <div className={cn("max-h-[26rem] overflow-y-auto scroll-area", className)}>
      <ul role="tree" className="space-y-0.5">
        {nodes.map((n) => (
          <Row key={n.id} node={n} siblings={nodes} depth={0} onAdd={onAdd} onEdit={onEdit} onDelete={onDelete} />
        ))}
      </ul>
    </div>
  )
}

function Row({
  node,
  siblings,
  depth,
  onAdd,
  onEdit,
  onDelete,
}: {
  node: EditableNode
  siblings: EditableNode[]
  depth: number
  onAdd: (parentId: string) => void
  onEdit: (node: EditableNode) => void
  onDelete: (node: EditableNode) => void
}) {
  const [open, setOpen] = React.useState(true)
  const hasChildren = node.children.length > 0
  const statusColor =
    node.status === "DISABLED"
      ? "bg-muted-foreground/40"
      : node.status === "ENABLED" || node.status === undefined
        ? "bg-primary"
        : "bg-muted-foreground/40"

  return (
    <li role="treeitem" aria-expanded={hasChildren ? open : undefined}>
      <div
        className={cn(
          "group flex items-center gap-2 rounded-lg px-2 py-2 transition-colors hover:bg-muted/60",
          depth > 0 && "ml-3 border-l border-border/60 pl-3"
        )}
      >
        {hasChildren ? (
          <button
            type="button"
            aria-label={open ? "折叠" : "展开"}
            onClick={() => setOpen((o) => !o)}
            className="flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-muted"
          >
            {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
          </button>
        ) : (
          <span className="w-5 shrink-0" />
        )}
        <span className={cn("size-1.5 shrink-0 rounded-full", statusColor)} aria-hidden />
        <Building2 className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate text-sm font-medium">{node.name}</span>
        {node.code && (
          <code className="hidden shrink-0 font-mono text-[11px] text-muted-foreground/60 sm:inline">{node.code}</code>
        )}
        {node.description && (
          <span className="ml-1 hidden truncate text-xs text-muted-foreground/60 md:inline md:max-w-[10rem]">
            {node.description}
          </span>
        )}

        <div className="ml-auto flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon" className="size-7" aria-label={`新增 ${node.name} 的子节点`} onClick={() => onAdd(node.id)}>
                <Plus className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>新增子节点</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon" className="size-7" aria-label={`编辑 ${node.name}`} onClick={() => onEdit(node)}>
                <Pencil className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>编辑</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-7 text-destructive hover:text-destructive"
                aria-label={`删除 ${node.name}`}
                onClick={() => onDelete(node)}
              >
                <Trash2 className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>删除(级联子节点)</TooltipContent>
          </Tooltip>
        </div>
      </div>
      {hasChildren && open && (
        <ul role="group" className="space-y-0.5">
          {node.children.map((c) => (
            <Row key={c.id} node={c} siblings={node.children} depth={depth + 1} onAdd={onAdd} onEdit={onEdit} onDelete={onDelete} />
          ))}
        </ul>
      )}
    </li>
  )
}
