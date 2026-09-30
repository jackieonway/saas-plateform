import * as React from "react"
import { ChevronDown, ChevronRight } from "lucide-react"
import { Checkbox } from "@/components/ui/checkbox"
import { AppIcon } from "@/lib/icons"
import { MENU_TYPE_META } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { AuthorizationMenu, MenuNode } from "@/api/types"

export interface TreeCheckNode {
  id: string
  parentId: string | null
  title: string
  type?: string
  icon?: string | null
  path?: string | null
  permission?: string | null
  sortOrder?: number
  children: TreeCheckNode[]
}

export function toTreeCheckNodes(nodes: MenuNode[]): TreeCheckNode[] {
  return nodes.map((n) => ({
    id: n.id,
    parentId: n.parentId,
    title: n.title,
    type: n.type,
    icon: n.icon,
    path: n.path,
    permission: n.permission,
    sortOrder: n.sortOrder,
    children: n.children ? toTreeCheckNodes(n.children) : [],
  }))
}

export function fromAuthMenus(flat: AuthorizationMenu[]): TreeCheckNode[] {
  return flat.map((m) => ({
    id: m.id,
    parentId: m.parentId,
    title: m.title ?? m.id.slice(0, 8),
    type: m.type,
    icon: m.icon ?? null,
    path: m.path ?? null,
    sortOrder: m.sortOrder,
    children: [],
  }))
}

function collectIds(nodes: TreeCheckNode[], out: string[] = []): string[] {
  for (const n of nodes) {
    out.push(n.id)
    collectIds(n.children, out)
  }
  return out
}

export interface TreeCheckProps {
  nodes: TreeCheckNode[]
  checked: Set<string>
  onToggle: (ids: string[], checked: boolean) => void
  disabled?: boolean
  /** 收集全部 id 供外部全选/清空 */
  onIdsKnown?: (ids: string[]) => void
  className?: string
}

/**
 * 三态复选树:
 * - 父节点勾选 = 全选子级;半选 = indeterminate
 * - 勾选父级自动联动所有后代
 */
export function TreeCheck({ nodes, checked, onToggle, disabled, className }: TreeCheckProps) {
  return (
    <div className={cn("select-none text-sm", disabled && "pointer-events-none opacity-50", className)}>
      {nodes.map((n) => (
        <TreeCheckItem
          key={n.id}
          node={n}
          checked={checked}
          onToggle={onToggle}
          disabled={disabled}
          depth={0}
        />
      ))}
    </div>
  )
}

function TreeCheckItem({
  node,
  checked,
  onToggle,
  disabled,
  depth,
}: {
  node: TreeCheckNode
  checked: Set<string>
  onToggle: (ids: string[], checked: boolean) => void
  disabled?: boolean
  depth: number
}) {
  const [open, setOpen] = React.useState(true)
  const hasChildren = node.children.length > 0
  const childIds = React.useMemo(() => collectIds(node.children), [node.children])
  const checkedChildren = childIds.filter((id) => checked.has(id)).length
  // 父节点:全部子级+自身勾选才视为全选;自身或任一子级勾选为半选
  const state: "checked" | "indeterminate" | "unchecked" = hasChildren
    ? checkedChildren === childIds.length && checked.has(node.id)
      ? "checked"
      : checkedChildren > 0 || checked.has(node.id)
        ? "indeterminate"
        : "unchecked"
    : checked.has(node.id)
      ? "checked"
      : "unchecked"

  const handle = () => {
    const ids = [node.id, ...childIds]
    onToggle(ids, state !== "checked")
  }

  const typeMeta = node.type ? MENU_TYPE_META[node.type] : undefined

  return (
    <div>
      <div
        className={cn(
          "group flex items-center gap-2 rounded-md py-1.5 pr-2 transition-colors hover:bg-muted/60",
          depth > 0 && "ml-4 border-l border-border/70 pl-3"
        )}
        style={{ minHeight: 36 }}
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
        <Checkbox
          checked={state === "checked" ? true : state === "indeterminate" ? "indeterminate" : false}
          onCheckedChange={handle}
          disabled={disabled}
          aria-label={`选择 ${node.title}`}
        />
        {node.icon && <AppIcon name={node.icon} className="size-4 shrink-0 text-muted-foreground" />}
        <span className={cn("truncate", state !== "unchecked" && "font-medium")}>{node.title}</span>
        {typeMeta && (
          <span
            className={cn(
              "shrink-0 rounded border px-1.5 py-px text-[10px] leading-4",
              typeMeta.className
            )}
          >
            {typeMeta.label}
          </span>
        )}
        {node.path && (
          <span className="ml-auto shrink-0 font-mono text-xs text-muted-foreground/70">{node.path}</span>
        )}
      </div>
      {hasChildren && open && (
        <div>
          {node.children.map((c) => (
            <TreeCheckItem
              key={c.id}
              node={c}
              checked={checked}
              onToggle={onToggle}
              disabled={disabled}
              depth={depth + 1}
            />
          ))}
        </div>
      )}
    </div>
  )
}

/** 全选 / 清空 工具按钮组 */
export function TreeCheckToolbar({
  nodes,
  checked,
  onSet,
  disabled,
}: {
  nodes: TreeCheckNode[]
  checked: Set<string>
  onSet: (checked: Set<string>) => void
  disabled?: boolean
}) {
  const allIds = React.useMemo(() => collectIds(nodes), [nodes])
  const allChecked = allIds.length > 0 && allIds.every((id) => checked.has(id))
  return (
    <div className="flex items-center gap-2">
      <button
        type="button"
        disabled={disabled || allChecked}
        className="rounded-md border bg-background px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50"
        onClick={() => onSet(new Set([...checked, ...allIds]))}
      >
        全选
      </button>
      <button
        type="button"
        disabled={disabled || checked.size === 0}
        className="rounded-md border bg-background px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50"
        onClick={() => {
          const next = new Set(checked)
          for (const id of allIds) next.delete(id)
          onSet(next)
        }}
      >
        清空
      </button>
      <span className="text-xs text-muted-foreground tabular-nums">
        已选 {checked.size} / {allIds.length}
      </span>
    </div>
  )
}
