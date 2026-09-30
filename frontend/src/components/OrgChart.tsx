import * as React from "react"
import {
  Building2,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  FolderClosed,
  GitBranch,
  ListTree,
  Maximize,
  Network,
  Users,
  X,
  ZoomIn,
  ZoomOut,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { StatusBadge } from "@/components/StatusBadge"
import { EmptyState, ErrorState } from "@/components/states"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import type { OrgNode } from "@/api/types"

/* ------------------------------ 常量与工具 ------------------------------ */

const ZOOM_MIN = 0.4
const ZOOM_MAX = 1.6
const ZOOM_STEP = 0.1

/** 超大树阈值:组织总数超过后默认折叠至第 2 层(仅展开前两层) */
const AUTO_COLLAPSE_THRESHOLD = 30

function clampZoom(z: number): number {
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(z * 100) / 100))
}

/** 递归累计组织 / 部门 / 成员总数 */
export function orgStats(nodes: OrgNode[]): { orgs: number; depts: number; users: number } {
  let orgs = 0
  let depts = 0
  let users = 0
  const walk = (list: OrgNode[]) => {
    for (const n of list) {
      orgs += 1
      depts += n.departmentCount ?? 0
      users += n.userCount ?? 0
      if (n.children?.length) walk(n.children)
    }
  }
  walk(nodes)
  return { orgs, depts, users }
}

/** 统计每个节点的后代(下级组织)总数 */
function buildDescendantCounts(nodes: OrgNode[]): Map<string, number> {
  const map = new Map<string, number>()
  const count = (node: OrgNode): number => {
    let n = 0
    for (const c of node.children ?? []) n += 1 + count(c)
    map.set(node.id, n)
    return n
  }
  nodes.forEach(count)
  return map
}

/** 收集第 2 层及更深的有子节点(自动折叠:只展开到第 2 层;更深层预折叠, 展开父级时逐层释放) */
function collectAutoCollapsed(nodes: OrgNode[]): Set<string> {
  const out = new Set<string>()
  const walk = (list: OrgNode[], depth: number) => {
    for (const n of list) {
      if (n.children?.length) {
        if (depth >= 2) out.add(n.id)
        walk(n.children, depth + 1)
      }
    }
  }
  walk(nodes, 1)
  return out
}

/** 按阈值计算初始折叠状态:≤30 全展开, >30 折叠至第 2 层 */
function computeInitialCollapsed(nodes: OrgNode[]): { collapsed: Set<string>; auto: boolean } {
  if (orgStats(nodes).orgs > AUTO_COLLAPSE_THRESHOLD) {
    const collapsed = collectAutoCollapsed(nodes)
    return { collapsed, auto: collapsed.size > 0 }
  }
  return { collapsed: new Set(), auto: false }
}

/** 收集全部有子节点的节点 id(「全部折叠」用, 含根) */
function collectAllCollapsible(nodes: OrgNode[]): Set<string> {
  const out = new Set<string>()
  const walk = (list: OrgNode[]) => {
    for (const n of list) {
      if (n.children?.length) {
        out.add(n.id)
        walk(n.children)
      }
    }
  }
  walk(nodes)
  return out
}

/* ------------------------------------------------------------------ */
/* 架构图总览视图(连接线样式见 index.css 中 .org-chart 系列)         */
/* ------------------------------------------------------------------ */
export function OrgChartView({
  nodes,
  isLoading,
  isError,
  error,
  onRetry,
  hasTenant,
  selectedId,
  onSelect,
  onLocate,
}: {
  nodes: OrgNode[]
  isLoading: boolean
  isError: boolean
  error?: unknown
  onRetry: () => void
  hasTenant: boolean
  selectedId: string | null
  onSelect: (id: string) => void
  /** 切回树形管理并定位当前选中节点 */
  onLocate: () => void
}) {
  const stats = React.useMemo(() => orgStats(nodes), [nodes])
  const selected = React.useMemo(() => findOrg(nodes, selectedId), [nodes, selectedId])
  const descCounts = React.useMemo(() => buildDescendantCounts(nodes), [nodes])
  const showTree = !isLoading && !isError && hasTenant && nodes.length > 0

  /* ---------- 折叠状态(默认全部展开;大树自动折叠至第 2 层) ---------- */
  const [collapsedIds, setCollapsedIds] = React.useState<Set<string>>(new Set())
  /** 本次数据是否触发了自动折叠(统计条一次性提示用) */
  const [autoCollapsed, setAutoCollapsed] = React.useState(false)
  const [autoHintDismissed, setAutoHintDismissed] = React.useState(false)
  const toggleCollapse = React.useCallback((id: string) => {
    setCollapsedIds((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])
  // 数据变化(如切换租户)时重算折叠状态(大树重新触发自动折叠)
  React.useEffect(() => {
    const initial = computeInitialCollapsed(nodes)
    setCollapsedIds(initial.collapsed)
    setAutoCollapsed(initial.auto)
    setAutoHintDismissed(false)
  }, [nodes])

  const expandAll = React.useCallback(() => {
    setCollapsedIds(new Set())
    setAutoHintDismissed(true)
  }, [])
  const collapseAll = React.useCallback(() => {
    setCollapsedIds(collectAllCollapsible(nodes))
    setAutoHintDismissed(true)
  }, [nodes])

  /** 被折叠节点的后代总数(嵌套折叠不重复计数) */
  const collapsedTotal = React.useMemo(() => {
    let n = 0
    const walk = (list: OrgNode[]) => {
      for (const node of list) {
        if (collapsedIds.has(node.id)) n += descCounts.get(node.id) ?? 0
        else if (node.children?.length) walk(node.children)
      }
    }
    walk(nodes)
    return n
  }, [nodes, collapsedIds, descCounts])

  /* ---------- 缩放控制 ---------- */
  const [zoom, setZoom] = React.useState(1)
  const viewportRef = React.useRef<HTMLDivElement>(null)
  const rootsRef = React.useRef<HTMLUListElement>(null)
  /** 树的自然布局宽度(不受 transform 影响),用于换算滚动区与"适应" */
  const [treeWidth, setTreeWidth] = React.useState(0)

  React.useEffect(() => {
    if (!showTree) return
    const el = rootsRef.current
    if (!el) return
    const measure = () => setTreeWidth(el.offsetWidth)
    measure()
    if (typeof ResizeObserver === "undefined") return
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [showTree, nodes])

  const zoomIn = () => setZoom((z) => clampZoom(z + ZOOM_STEP))
  const zoomOut = () => setZoom((z) => clampZoom(z - ZOOM_STEP))
  /** 重置适应:按容器可用宽度与树的布局宽度计算比例(不放大超过 100%),clamp 到缩放范围 */
  const fitZoom = () => {
    const vp = viewportRef.current
    const roots = rootsRef.current
    if (!vp || !roots) return
    const cs = window.getComputedStyle(vp)
    const pad = parseFloat(cs.paddingLeft || "0") + parseFloat(cs.paddingRight || "0")
    const avail = vp.clientWidth - pad
    const natural = roots.offsetWidth
    if (!avail || !natural) return
    setZoom(clampZoom(Math.min(1, avail / natural)))
  }

  // Ctrl + 滚轮缩放(需 preventDefault,监听必须非 passive)
  React.useEffect(() => {
    const vp = viewportRef.current
    if (!vp) return
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return
      e.preventDefault()
      setZoom((z) => clampZoom(z - e.deltaY * 0.002))
    }
    vp.addEventListener("wheel", onWheel, { passive: false })
    return () => vp.removeEventListener("wheel", onWheel)
  }, [])

  return (
    <div className="space-y-4">
      {/* 顶部统计条(折叠时仍显示全量数字) */}
      <div className="flex items-stretch overflow-hidden rounded-xl border bg-card">
        <div className="grid flex-1 grid-cols-3 divide-x">
          <StatItem icon={<Network className="size-4" />} label="组织总数" value={stats.orgs} />
          <StatItem icon={<Building2 className="size-4" />} label="部门总数" value={stats.depts} />
          <StatItem icon={<Users className="size-4" />} label="成员总数" value={stats.users} />
        </div>
        {collapsedTotal > 0 && (
          <div className="hidden items-center gap-1.5 border-l px-4 text-xs text-muted-foreground md:flex">
            <FolderClosed className="size-3.5 shrink-0 text-primary/70" />
            <span>
              已折叠 <b className="tabular-nums text-foreground">{collapsedTotal}</b> 个组织
            </span>
          </div>
        )}
      </div>

      {/* 自动折叠一次性提示(可关闭) */}
      {autoCollapsed && !autoHintDismissed && (
        <div
          role="status"
          className="flex items-center gap-2 rounded-xl border border-dashed border-primary/35 bg-primary/[0.04] px-3.5 py-2 text-xs text-muted-foreground animate-in fade-in slide-in-from-top-1 duration-300"
        >
          <FolderClosed className="size-3.5 shrink-0 text-primary/70" />
          <span className="min-w-0 flex-1">
            当前组织树共 <b className="tabular-nums text-foreground">{stats.orgs}</b> 个节点,大树已自动折叠至 2 层;可点击节点下方箭头逐层展开,或
            <button type="button" onClick={expandAll} className="mx-0.5 font-medium text-primary hover:underline">
              全部展开
            </button>
          </span>
          <button
            type="button"
            onClick={() => setAutoHintDismissed(true)}
            aria-label="关闭自动折叠提示"
            title="关闭提示"
            className="flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground"
          >
            <X className="size-3.5" />
          </button>
        </div>
      )}

      {/* 架构图画布 */}
      <div className="rounded-xl border bg-card">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b px-4 py-3">
          <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">组织架构总览</p>
          <div className="flex items-center gap-3">
            <p className="hidden text-xs text-muted-foreground/70 md:block">
              点击节点查看摘要 · Ctrl+滚轮缩放
            </p>
            {/* 全部展开 / 全部折叠 */}
            <div className="flex items-center gap-0.5 rounded-lg border bg-background p-0.5">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7 rounded-md"
                    onClick={expandAll}
                    aria-label="全部展开"
                    title="全部展开"
                  >
                    <ChevronsUpDown className="size-3.5" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>全部展开</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7 rounded-md"
                    onClick={collapseAll}
                    aria-label="全部折叠"
                    title="全部折叠"
                  >
                    <ChevronsDownUp className="size-3.5" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>全部折叠</TooltipContent>
              </Tooltip>
            </div>
            {/* 缩放控件 */}
            <div className="flex items-center gap-0.5 rounded-lg border bg-background p-0.5">
              <Button
                variant="ghost"
                size="icon"
                className="size-7 rounded-md"
                onClick={zoomOut}
                disabled={zoom <= ZOOM_MIN}
                aria-label="缩小架构图"
                title="缩小"
              >
                <ZoomOut className="size-3.5" />
              </Button>
              <span
                className="w-11 text-center text-xs font-medium tabular-nums text-muted-foreground"
                aria-live="polite"
              >
                {Math.round(zoom * 100)}%
              </span>
              <Button
                variant="ghost"
                size="icon"
                className="size-7 rounded-md"
                onClick={zoomIn}
                disabled={zoom >= ZOOM_MAX}
                aria-label="放大架构图"
                title="放大"
              >
                <ZoomIn className="size-3.5" />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                className="size-7 rounded-md"
                onClick={fitZoom}
                aria-label="重置并适应容器宽度"
                title="重置适应"
              >
                <Maximize className="size-3.5" />
              </Button>
            </div>
          </div>
        </div>
        <div ref={viewportRef} className="scroll-area max-h-[36rem] overflow-x-auto overflow-y-auto p-6">
          {isLoading ? (
            <div className="mx-auto flex max-w-3xl flex-col items-center gap-6 py-2">
              <Skeleton className="h-[5.5rem] w-56 rounded-xl" />
              <div className="flex gap-8">
                {Array.from({ length: 3 }).map((_, i) => (
                  <div key={i} className="flex flex-col items-center gap-6">
                    <div className="h-6 w-px bg-border" />
                    <Skeleton className="h-[5.5rem] w-[13.5rem] rounded-xl" />
                  </div>
                ))}
              </div>
            </div>
          ) : isError ? (
            <ErrorState
              className="border-0 bg-transparent"
              message={error instanceof Error ? error.message : undefined}
              onRetry={onRetry}
            />
          ) : !hasTenant ? (
            <EmptyState
              className="py-16"
              title="请先选择租户"
              description="顶部选择租户后,将展示其完整组织架构"
              icon={<Network className="size-6" />}
            />
          ) : nodes.length === 0 ? (
            <EmptyState
              className="py-16"
              title="该租户暂无组织"
              description="先在树形管理中创建根组织,再回到这里查看架构图"
              icon={<Building2 className="size-6" />}
              action={
                <Button size="sm" variant="outline" onClick={onLocate}>
                  <ListTree className="size-4" />
                  切换到树形管理
                </Button>
              }
            />
          ) : (
            /* 尺寸换算层:宽度 = 树布局宽 × 缩放比,保证缩放后滚动范围与视觉一致 */
            <div
              className="w-max min-w-full"
              style={treeWidth > 0 ? { width: Math.round(treeWidth * zoom) } : undefined}
            >
              {/* 缩放画布:origin 取 top center,配合 mx-auto 使树在缩放后仍水平居中 */}
              <div
                className="org-chart mx-auto w-max"
                style={{ transform: `scale(${zoom})`, transformOrigin: "top center" }}
              >
                <ul ref={rootsRef} className="org-roots w-max">
                  {nodes.map((n) => (
                    <Branch
                      key={n.id}
                      node={n}
                      isRoot
                      selectedId={selectedId}
                      onSelect={onSelect}
                      collapsedIds={collapsedIds}
                      descCounts={descCounts}
                      onToggle={toggleCollapse}
                    />
                  ))}
                </ul>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* 选中组织摘要卡 */}
      {selected && (
        <div className="rounded-xl border bg-card p-5">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0 space-y-2">
              <div className="flex flex-wrap items-center gap-2">
                <h2 className="text-lg font-semibold">{selected.name}</h2>
                <StatusBadge status={selected.status} />
                <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-muted-foreground">
                  {selected.code}
                </code>
                {!selected.parentId && (
                  <span className="rounded-md border border-primary/30 bg-primary/10 px-1.5 py-0.5 text-[11px] font-medium text-primary">
                    根组织
                  </span>
                )}
              </div>
              <p className="max-w-2xl text-sm text-muted-foreground">{selected.description || "暂无描述"}</p>
              <div className="flex flex-wrap gap-x-5 gap-y-1 text-xs text-muted-foreground">
                <span>
                  直属部门 <b className="tabular-nums text-foreground">{selected.departmentCount}</b>
                </span>
                <span>
                  直属成员 <b className="tabular-nums text-foreground">{selected.userCount}</b>
                </span>
                <span>
                  下级组织 <b className="tabular-nums text-foreground">{selected.children?.length ?? 0}</b>
                </span>
              </div>
            </div>
            <Button variant="outline" size="sm" className="shrink-0" onClick={onLocate}>
              <ListTree className="size-4" />
              在树形管理中定位
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

/* ------------------------------ 内部实现 ------------------------------ */

function StatItem({ icon, label, value }: { icon: React.ReactNode; label: string; value: number }) {
  return (
    <div className="flex items-center gap-3 px-4 py-3.5">
      <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
        {icon}
      </span>
      <div className="min-w-0">
        <p className="text-2xl font-semibold leading-none tabular-nums">{value}</p>
        <p className="mt-1 truncate text-xs text-muted-foreground">{label}</p>
      </div>
    </div>
  )
}

function findOrg(nodes: OrgNode[], id: string | null): OrgNode | null {
  if (!id) return null
  for (const n of nodes) {
    if (n.id === id) return n
    const sub = n.children?.length ? findOrg(n.children, id) : null
    if (sub) return sub
  }
  return null
}

/** 架构图分支:li 节点 + 卡片 + 子级 ul(折叠时隐藏整棵子树) */
function Branch({
  node,
  isRoot,
  selectedId,
  onSelect,
  collapsedIds,
  descCounts,
  onToggle,
}: {
  node: OrgNode
  isRoot?: boolean
  selectedId: string | null
  onSelect: (id: string) => void
  collapsedIds: Set<string>
  descCounts: Map<string, number>
  onToggle: (id: string) => void
}) {
  const active = node.id === selectedId
  const collapsed = collapsedIds.has(node.id)
  return (
    <li className={cn("org-branch", active && "is-active")}>
      <span className="org-stub" aria-hidden />
      <ChartNode
        node={node}
        isRoot={isRoot}
        active={active}
        onSelect={onSelect}
        collapsed={collapsed}
        descCount={descCounts.get(node.id) ?? 0}
        onToggle={onToggle}
      />
      {!collapsed && node.children?.length ? (
        <ul className="org-children">
          {node.children.map((c) => (
            <Branch
              key={c.id}
              node={c}
              selectedId={selectedId}
              onSelect={onSelect}
              collapsedIds={collapsedIds}
              descCounts={descCounts}
              onToggle={onToggle}
            />
          ))}
        </ul>
      ) : null}
    </li>
  )
}

/** 架构图节点卡片(带折叠切换钮;按钮置于卡片外部,避免嵌套交互元素) */
function ChartNode({
  node,
  isRoot,
  active,
  onSelect,
  collapsed,
  descCount,
  onToggle,
}: {
  node: OrgNode
  isRoot?: boolean
  active: boolean
  onSelect: (id: string) => void
  collapsed: boolean
  descCount: number
  onToggle: (id: string) => void
}) {
  const enabled = node.status === "ENABLED"
  const hasChildren = (node.children?.length ?? 0) > 0
  const card = (
    <button
      type="button"
      onClick={() => onSelect(node.id)}
      aria-pressed={active}
      title={node.description ? `${node.name} · ${node.description}` : node.name}
      className={cn(
        "block w-full cursor-pointer rounded-[11px] bg-card p-3.5 text-left outline-none transition-all duration-200",
        "hover:-translate-y-1 hover:shadow-xl hover:shadow-primary/15 focus-visible:ring-[3px] focus-visible:ring-ring/50",
        active ? "ring-2 ring-primary/60" : isRoot ? "" : "ring-0",
        !enabled && "saturate-[0.6]"
      )}
    >
      <span className="block space-y-2">
        <span className="flex items-center gap-2">
          <span
            aria-hidden
            className={cn(
              "size-2 shrink-0 rounded-full",
              enabled
                ? "bg-primary shadow-[0_0_0_3px_color-mix(in_oklch,var(--primary)_20%,transparent)]"
                : "bg-muted-foreground/40"
            )}
          />
          <span
            className={cn(
              "min-w-0 truncate leading-tight",
              isRoot ? "text-[15px] font-semibold" : "text-sm font-semibold"
            )}
          >
            {node.name}
          </span>
        </span>
        <span className="block truncate font-mono text-[10px] tracking-wide text-muted-foreground/80">
          {node.code}
        </span>
        <span className="flex flex-wrap items-center gap-1.5">
          <span className="inline-flex items-center gap-1 rounded-md bg-muted px-1.5 py-0.5 text-[11px] tabular-nums text-muted-foreground">
            <Building2 className="size-3 text-primary/70" />
            {node.departmentCount} 部门
          </span>
          <span className="inline-flex items-center gap-1 rounded-md bg-muted px-1.5 py-0.5 text-[11px] tabular-nums text-muted-foreground">
            <Users className="size-3 text-primary/70" />
            {node.userCount} 成员
          </span>
          {hasChildren && !collapsed && (
            <span className="inline-flex items-center gap-0.5 rounded-md bg-primary/10 px-1.5 py-0.5 text-[11px] tabular-nums text-primary">
              <GitBranch className="size-3" />
              {node.children!.length} 下级
            </span>
          )}
          {hasChildren && collapsed && descCount > 0 && (
            <span className="inline-flex items-center gap-0.5 rounded-md bg-primary px-1.5 py-0.5 text-[11px] font-medium tabular-nums text-primary-foreground shadow-sm">
              <FolderClosed className="size-3" />+{descCount}
            </span>
          )}
        </span>
      </span>
    </button>
  )

  return (
    <div
      className={cn(
        "relative",
        isRoot
          ? "w-60 rounded-xl bg-gradient-to-b from-primary/80 via-primary/45 to-primary/15 p-[1.5px] shadow-lg shadow-primary/20"
          : "w-[13.5rem] rounded-xl border border-border/80 shadow-sm"
      )}
    >
      {card}
      {hasChildren && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation()
            onToggle(node.id)
          }}
          aria-expanded={!collapsed}
          aria-label={collapsed ? `展开 ${node.name} 的 ${descCount} 个下级组织` : `折叠 ${node.name} 的下级组织`}
          title={collapsed ? `展开 ${descCount} 个下级组织` : "折叠下级组织"}
          className={cn(
            "absolute -bottom-2.5 left-1/2 z-10 flex size-5 -translate-x-1/2 items-center justify-center rounded-full border bg-background shadow-sm outline-none transition-colors",
            "hover:border-primary/60 hover:text-primary focus-visible:ring-2 focus-visible:ring-ring",
            active ? "border-primary/60 text-primary" : "border-border text-muted-foreground"
          )}
        >
          {collapsed ? <ChevronRight className="size-3" /> : <ChevronDown className="size-3" />}
        </button>
      )}
    </div>
  )
}
