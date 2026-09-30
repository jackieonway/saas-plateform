import * as React from "react"
import { useNavigate } from "react-router-dom"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useVirtualizer } from "@tanstack/react-virtual"
import {
  ArrowRight,
  Bell,
  Building2,
  Check,
  CheckCheck,
  CircleCheck,
  CircleHelp,
  Info,
  Keyboard,
  Landmark,
  ListChecks,
  ListFilter,
  Loader2,
  Megaphone,
  Moon,
  MoveVertical,
  RefreshCw,
  RotateCcw,
  Search,
  SearchX,
  Send,
  Settings2,
  ShieldAlert,
  Trash2,
  TriangleAlert,
  User,
  X,
} from "lucide-react"
import type { LucideIcon } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Badge } from "@/components/ui/badge"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Switch } from "@/components/ui/switch"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { PageHeader } from "@/components/layout/PageHeader"
import { DataPagination } from "@/components/DataPagination"
import { EntityTreePicker, type PickerNode } from "@/components/EntityTreePicker"
import { TableDensityToggle } from "@/components/TableDensityToggle"
import { ConfirmDialog } from "@/components/ConfirmDialog"
import { EmptyState, ErrorState } from "@/components/states"
import { Skeleton } from "@/components/ui/skeleton"
import { notificationsApi, type NotificationPrefs } from "@/api/notifications"
import { usersApi } from "@/api/users"
import { organizationsApi } from "@/api/organizations"
import { tenantsApi } from "@/api/tenants"
import type { NotificationRecord, NotificationType } from "@/api/types"
import { useAuthStore } from "@/stores/auth"
import { usePermissionsQuery } from "@/hooks/use-auth-data"
import { useTableDensity } from "@/hooks/use-table-density"
import { useNotificationHotkeys } from "@/hooks/use-notification-hotkeys"
import { asArray, asPage } from "@/lib/api"
import { formatDateTime, formatRelative, useDebounced } from "@/lib/format"
import { cn } from "@/lib/utils"

/* ============ 类型视觉映射 ============ */

const TYPE_META: Record<
  NotificationType,
  { icon: typeof Info; label: string; chip: string }
> = {
  INFO: {
    icon: Info,
    label: "通知",
    chip: "border-primary/25 bg-primary/8 text-primary",
  },
  SUCCESS: {
    icon: CircleCheck,
    label: "成功",
    chip: "border-emerald-500/25 bg-emerald-500/8 text-emerald-600 dark:text-emerald-400",
  },
  WARNING: {
    icon: TriangleAlert,
    label: "警告",
    chip: "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400",
  },
  SECURITY: {
    icon: ShieldAlert,
    label: "安全",
    chip: "border-red-500/25 bg-red-500/8 text-red-600 dark:text-red-400",
  },
}

/** 类型筛选 chips:全部 + 四色类型 (选中时复用 TYPE_META 配色) */
const TYPE_FILTER_OPTIONS: { value: NotificationType | "ALL"; label: string }[] = [
  { value: "ALL", label: "全部" },
  { value: "INFO", label: "信息" },
  { value: "SUCCESS", label: "成功" },
  { value: "WARNING", label: "警告" },
  { value: "SECURITY", label: "安全" },
]

/* ============ 虚拟滚动阈值 ============ */

/**
 * 本页行数超过该阈值时启用虚拟渲染(useVirtualizer 仅挂载可视区行, 允许 100/200 大页尺寸);
 * 未超过则走原有全量渲染路径 —— 小列表的键盘导航/选择/批量操作行为零变化。
 */
const VIRTUAL_THRESHOLD = 60

/* ============ 搜索命中词高亮 ============ */

/** 转义正则特殊字符, 保证 query 始终按字面量子串匹配 */
function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}

/**
 * 命中词高亮: query 非空时对 text 做不区分大小写子串匹配, 命中片段包主题化 <mark>;
 * 空 query 零开销直返原文。安全: split 分段渲染纯 React 节点(文本天然转义, 无 innerHTML),
 * 多处命中全部高亮且保留原文大小写; memo 使翻页/选中态等无关重渲染下输出稳定。
 */
const Highlight = React.memo(function Highlight({ text, query }: { text: string; query: string }) {
  const q = query.trim()
  if (!q) return text
  const qLower = q.toLowerCase()
  const parts = text.split(new RegExp(`(${escapeRegExp(q)})`, "ig"))
  return (
    <>
      {parts.map((part, i) =>
        part.toLowerCase() === qLower ? (
          <mark key={i} className="rounded-[3px] bg-primary/15 px-0.5 font-medium text-primary">
            {part}
          </mark>
        ) : (
          <span key={i}>{part}</span>
        )
      )}
    </>
  )
})

export default function NotificationsPage() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [tab, setTab] = React.useState<"UNREAD" | "ALL">("UNREAD")
  const [typeFilter, setTypeFilter] = React.useState<NotificationType | "ALL">("ALL")
  const [keyword, setKeyword] = React.useState("")
  const debouncedQ = useDebounced(keyword.trim(), 300)
  const [page, setPage] = React.useState(1)
  const [size, setSize] = React.useState(10)
  const [sendOpen, setSendOpen] = React.useState(false)
  const [prefsOpen, setPrefsOpen] = React.useState(false)
  /** ? 键 / 提示条入口打开的快捷键帮助 Dialog */
  const [helpOpen, setHelpOpen] = React.useState(false)
  const [batchDeleteOpen, setBatchDeleteOpen] = React.useState(false)
  /** 当前页选中的通知 id (翻页/切筛选自动清空, 避免跨页误操作) */
  const [selected, setSelected] = React.useState<Set<string>>(new Set())
  const { density } = useTableDensity()
  const compact = density === "compact"

  const permissionsQuery = usePermissionsQuery()
  const permissions = new Set(permissionsQuery.data?.permissions ?? [])
  const isSuper = useAuthStore((s) => s.profile?.user.isSuperAdmin) ?? false
  const canManage = isSuper || permissions.has("system:user")

  const listQuery = useQuery({
    queryKey: ["notifications", tab, typeFilter, debouncedQ, page, size],
    queryFn: () =>
      notificationsApi.list({
        filter: tab,
        type: typeFilter === "ALL" ? undefined : typeFilter,
        q: debouncedQ || undefined,
        page,
        size,
      }),
    staleTime: 15_000,
    retry: 1,
    placeholderData: (prev) => prev,
  })
  const countQuery = useQuery({
    queryKey: ["notifications", "unread-count"],
    queryFn: () => notificationsApi.unreadCount(),
    staleTime: 15_000,
    retry: 1,
  })
  React.useEffect(() => setPage(1), [tab, typeFilter, debouncedQ, size])
  /* 翻页 / 切筛选 / 搜索时清空选择 */
  React.useEffect(() => {
    setSelected(new Set())
  }, [page, tab, typeFilter, debouncedQ, size])

  /* useCallback: 作为 memo 行组件 props 的回调链保持引用稳定, 避免整表 memo 失效 */
  const invalidate = React.useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: ["notifications"] })
  }, [queryClient])

  const readAllMutation = useMutation({
    mutationFn: () => notificationsApi.readAll(),
    onSuccess: (d) => {
      toast.success(d.updated > 0 ? `已将 ${d.updated} 条通知标记为已读` : "没有未读通知")
      invalidate()
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "操作失败"),
  })

  const removeMutation = useMutation({
    mutationFn: (id: string) => notificationsApi.remove(id),
    onSuccess: () => {
      toast.success("通知已删除")
      invalidate()
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "删除失败"),
  })

  /* ---- 批量已读 / 批量删除 ---- */
  const readBatchMutation = useMutation({
    mutationFn: (ids: string[]) => notificationsApi.readBatch(ids),
    onSuccess: (d) => {
      toast.success(`已将 ${d.updated} 条通知标记为已读`)
      setSelected(new Set())
      invalidate()
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "操作失败"),
  })
  const removeBatchMutation = useMutation({
    mutationFn: (ids: string[]) => notificationsApi.removeBatch(ids),
    onSuccess: (d) => {
      toast.success(`已删除 ${d.deleted} 条通知`)
      setSelected(new Set())
      setBatchDeleteOpen(false)
      invalidate()
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "删除失败"),
  })

  const records = listQuery.data?.records ?? []
  const total = listQuery.data?.total ?? 0
  const unread = countQuery.data?.count ?? 0

  /* 记录集变化时把选择裁剪到当前页(已读/被删的行自动移出选择) */
  React.useEffect(() => {
    setSelected((prev) => {
      if (prev.size === 0) return prev
      const ids = new Set(records.map((n) => n.id))
      const next = new Set([...prev].filter((id) => ids.has(id)))
      return next.size === prev.size ? prev : next
    })
  }, [listQuery.data])

  const pageIds = records.map((n) => n.id)
  const selectedOnPage = pageIds.filter((id) => selected.has(id))
  const allSelected = pageIds.length > 0 && selectedOnPage.length === pageIds.length
  const someSelected = selectedOnPage.length > 0 && !allSelected
  const pageUnread = records.filter((n) => !n.isRead).length

  const toggleOne = React.useCallback((id: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])
  const selectAllOnPage = () => setSelected(new Set(pageIds))
  const clearSelection = () => setSelected(new Set())

  /** 批量已读:仅未读行参与;若过滤后为空提示 */
  const doBatchRead = () => {
    const unreadIds = records.filter((n) => selected.has(n.id) && !n.isRead).map((n) => n.id)
    if (unreadIds.length === 0) {
      toast.info("所选均已是已读状态")
      return
    }
    readBatchMutation.mutate(unreadIds)
  }
  const doBatchDelete = () => {
    const ids = records.filter((n) => selected.has(n.id)).map((n) => n.id)
    if (ids.length === 0) return
    removeBatchMutation.mutate(ids)
  }

  /* ---- 键盘快捷键(J/K 移动 · Space 选择 · X 批量已读 · Esc 取消 · ? 帮助;仅桌面生效) ---- */
  const { focusIndex, resetFocus } = useNotificationHotkeys({
    // 不含 records.length > 0: 空列表时仍允许 ? 打开帮助(行操作类按键由 hook 内 count 早退拦截)
    enabled:
      !listQuery.isLoading &&
      !listQuery.isError &&
      !sendOpen &&
      !prefsOpen &&
      !helpOpen &&
      !batchDeleteOpen,
    count: records.length,
    handlers: {
      onToggleSelect: (index) => {
        const id = records[index]?.id
        if (id !== undefined) toggleOne(id)
      },
      onMarkSelectedRead: (focusedIndex) => {
        // 优先批量已读当前选中的未读行(复用 readBatch);无选中时对焦点行单条已读
        const unreadIds = records.filter((n) => selected.has(n.id) && !n.isRead).map((n) => n.id)
        if (unreadIds.length > 0) {
          readBatchMutation.mutate(unreadIds)
          return
        }
        const focused = records[focusedIndex]
        if (focused && !focused.isRead) {
          void notificationsApi.markRead(focused.id).then(invalidate)
          return
        }
        toast.info(selected.size > 0 ? "所选均已是已读状态" : "焦点行已是已读状态")
      },
      onClearSelection: clearSelection,
      onOpenHelp: () => setHelpOpen(true),
    },
  })

  /* 焦点行变化时保持在可视区(nearest, 不扰动滚动位置) */
  const listRef = React.useRef<HTMLUListElement>(null)

  /* ---- 虚拟滚动兜底: 本页行数超过阈值时仅挂载可视区行(+overscan) ---- */
  const virtualOn = records.length > VIRTUAL_THRESHOLD
  const scrollRef = React.useRef<HTMLUListElement>(null)
  const rowVirtualizer = useVirtualizer<HTMLUListElement, HTMLLIElement>({
    count: records.length,
    getScrollElement: () => scrollRef.current,
    // 行高随内容 line-clamp-2 实际行数与 compact 密度浮动: 起估值仅兜底, 实际行高以 measureElement 动态测量为准
    estimateSize: () => (compact ? 56 : 96),
    overscan: 6,
    getItemKey: (i) => records[i]?.id ?? i,
    enabled: virtualOn,
  })

  React.useEffect(() => {
    if (focusIndex < 0) return
    if (virtualOn) {
      /* 虚拟模式: li 不全量挂载, querySelector 命不中 → 交给 virtualizer 按偏移定位
         (align "auto" = 已可见不动/最少滚动, 等价原 scrollIntoView block:"nearest" 语义) */
      rowVirtualizer.scrollToIndex(focusIndex, { align: "auto" })
    } else {
      listRef.current
        ?.querySelector(`li[data-notify-row="${focusIndex}"]`)
        ?.scrollIntoView({ block: "nearest" })
    }
  }, [focusIndex, virtualOn, rowVirtualizer])

  /* 翻页 / 切筛选 / 搜索时焦点复位(与选择清空同步) */
  React.useEffect(() => {
    resetFocus()
  }, [page, tab, typeFilter, debouncedQ, size, resetFocus])

  const openItem = React.useCallback(
    (n: NotificationRecord) => {
      if (!n.isRead) {
        void notificationsApi.markRead(n.id).then(invalidate)
      }
      if (n.link) navigate(n.link)
    },
    [invalidate, navigate]
  )

  /** 行内「标记已读」(沿用原内联 onClick 语义: 静默吞错) */
  const markReadOne = React.useCallback(
    (id: string) => {
      notificationsApi.markRead(id).then(invalidate).catch(() => undefined)
    },
    [invalidate]
  )

  /** 行内「删除」(react-query 的 mutate 引用稳定, 行组件 memo 不因此失效) */
  const removeOne = React.useCallback(
    (id: string) => {
      removeMutation.mutate(id)
    },
    [removeMutation.mutate]
  )

  const typeFilterLabel = TYPE_FILTER_OPTIONS.find((o) => o.value === typeFilter)?.label ?? "全部"

  return (
    <div>
      <PageHeader
        title="通知中心"
        description="账号安全、授权变更与平台公告的统一收件箱"
        icon={<Bell className="size-5" />}
        actions={
          <>
            {canManage && (
              <Button size="sm" onClick={() => setSendOpen(true)}>
                <Megaphone className="size-4" />
                发送通知
              </Button>
            )}
            <Tooltip>
              <TooltipTrigger asChild>
                <Button variant="outline" size="sm" onClick={() => setPrefsOpen(true)}>
                  <Settings2 className="size-4" />
                  通知设置
                </Button>
              </TooltipTrigger>
              <TooltipContent>免打扰时段与类型静音偏好</TooltipContent>
            </Tooltip>
            <Button
              variant="outline"
              size="sm"
              onClick={() => readAllMutation.mutate()}
              disabled={readAllMutation.isPending || unread === 0}
            >
              {readAllMutation.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <CheckCheck className="size-4" />
              )}
              全部已读
            </Button>
          </>
        }
      />

      {/* 概览条 */}
      <div className="mb-4 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border bg-card px-4 py-3">
        <Tabs value={tab} onValueChange={(v) => setTab(v as "UNREAD" | "ALL")}>
          <TabsList aria-label="通知筛选">
            <TabsTrigger value="UNREAD" className="gap-1.5">
              未读
              <Badge
                variant="secondary"
                className={cn(
                  "h-5 min-w-5 px-1 tabular-nums",
                  unread > 0 && "bg-primary/15 text-primary"
                )}
              >
                {unread}
              </Badge>
            </TabsTrigger>
            <TabsTrigger value="ALL">全部</TabsTrigger>
          </TabsList>
        </Tabs>

        {/* 类型筛选 chips */}
        <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="类型筛选">
          <ListFilter className="mr-0.5 size-3.5 text-muted-foreground/70" />
          {TYPE_FILTER_OPTIONS.map((o) => {
            const active = typeFilter === o.value
            const Icon = o.value === "ALL" ? null : TYPE_META[o.value as NotificationType].icon
            return (
              <button
                key={o.value}
                type="button"
                onClick={() => setTypeFilter(o.value)}
                aria-pressed={active}
                title={o.value === "ALL" ? "全部类型" : `仅看${o.label}类型`}
                className={cn(
                  "inline-flex items-center gap-1 rounded-full border px-2.5 py-1 text-xs font-medium transition-colors",
                  active
                    ? o.value === "ALL"
                      ? "border-primary/40 bg-primary/10 text-primary"
                      : TYPE_META[o.value as NotificationType].chip
                    : "border-border bg-background text-muted-foreground hover:bg-accent hover:text-foreground"
                )}
              >
                {Icon && <Icon className="size-3.5" />}
                {o.label}
              </button>
            )
          })}
        </div>

        {/* 搜索框:与 Tabs / 类型 chips 组合查询(300ms debounce) */}
        <div className="relative min-w-0 flex-1 basis-52 sm:max-w-xs">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="搜索通知标题或内容…"
            aria-label="搜索通知"
            className="pr-8 pl-9"
          />
          {keyword && (
            <button
              type="button"
              onClick={() => setKeyword("")}
              aria-label="清除搜索"
              title="清除搜索"
              className="absolute right-2 top-1/2 flex size-5 -translate-y-1/2 items-center justify-center rounded-full text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
            >
              <X className="size-3.5" />
            </button>
          )}
        </div>

        <Button
          variant="ghost"
          size="icon"
          className="size-8 text-muted-foreground"
          onClick={() => {
            void listQuery.refetch()
            void countQuery.refetch()
          }}
          aria-label="刷新"
        >
          {listQuery.isFetching ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <RefreshCw className="size-4" />
          )}
        </Button>

        <div className="ml-auto flex items-center gap-2">
          <span className="text-xs text-muted-foreground">
            共 <span className="tabular-nums">{total}</span> 条
            {typeFilter !== "ALL" && ` · ${typeFilterLabel}`}
            {tab === "UNREAD" ? "未读" : "通知"}
            {debouncedQ && (
              <>
                {" · "}
                <span className="max-w-[7rem] truncate align-middle text-foreground/70">「{debouncedQ}」</span>
              </>
            )}
          </span>
          <TableDensityToggle />
        </div>
      </div>

      {/* 批量操作栏:选中 > 0 时浮现 */}
      {selected.size > 0 && (
        <div
          role="toolbar"
          aria-label="批量操作"
          className="mb-3 flex flex-wrap items-center gap-2 rounded-xl border bg-muted/70 px-3 py-2 shadow-sm backdrop-blur animate-in fade-in slide-in-from-top-2 duration-200"
        >
          <span className="inline-flex items-center gap-2 text-sm font-medium">
            <span className="flex size-5 items-center justify-center rounded-md bg-primary text-primary-foreground">
              <Check className="size-3" />
            </span>
            已选 <span className="tabular-nums">{selected.size}</span> 条
          </span>
          <span className="h-5 w-px bg-border" aria-hidden />
          <Button
            variant="outline"
            size="sm"
            onClick={doBatchRead}
            disabled={readBatchMutation.isPending}
          >
            {readBatchMutation.isPending ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <CheckCheck className="size-4" />
            )}
            批量已读
          </Button>
          <Button
            variant="destructive"
            size="sm"
            onClick={() => setBatchDeleteOpen(true)}
            disabled={removeBatchMutation.isPending}
          >
            <Trash2 className="size-4" />
            批量删除
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="ml-auto size-11 text-muted-foreground hover:text-foreground"
            onClick={clearSelection}
            aria-label="取消选择"
            title="取消选择"
          >
            <X className="size-4" />
          </Button>
        </div>
      )}

      {/* 通知列表 */}
      <div className="overflow-hidden rounded-xl border bg-card">
        {listQuery.isLoading ? (
          <div className="space-y-1 p-4">
            {Array.from({ length: 5 }).map((_, i) => (
              <div key={i} className="flex items-start gap-3.5 py-2.5">
                <Skeleton className="size-9 shrink-0 rounded-xl" />
                <div className="flex-1 space-y-2">
                  <Skeleton className="h-4 w-2/5" />
                  <Skeleton className="h-3 w-4/5" />
                  <Skeleton className="h-3 w-1/4" />
                </div>
              </div>
            ))}
          </div>
        ) : listQuery.isError ? (
          <ErrorState
            className="m-2 border-0 bg-transparent"
            message={listQuery.error instanceof Error ? listQuery.error.message : undefined}
            onRetry={() => listQuery.refetch()}
          />
        ) : records.length === 0 ? (
          debouncedQ ? (
            <EmptyState
              icon={<SearchX className="size-8" />}
              title={
                <>
                  未找到与「<span className="text-foreground">{debouncedQ}</span>」相关的通知
                </>
              }
              description="换个关键词试试,或清除搜索查看全部通知"
              action={
                <Button size="sm" variant="outline" onClick={() => setKeyword("")}>
                  <X className="size-4" />
                  清除搜索
                </Button>
              }
            />
          ) : (
            <EmptyState
              icon={<CheckCheck className="size-8" />}
              title={tab === "UNREAD" ? "没有未读通知" : "暂无通知"}
              description={
                tab === "UNREAD"
                  ? "所有通知均已处理,保持干干净净"
                  : "密码重置、角色变更、设备授权等事件将在这里提醒您"
              }
            />
          )
        ) : (
          <>
            {/* 全选表头 */}
            <div className="flex items-center gap-2.5 border-b bg-muted/30 px-4 py-2 sm:px-5">
              <Checkbox
                checked={allSelected ? true : someSelected ? "indeterminate" : false}
                onCheckedChange={() => (allSelected ? clearSelection() : selectAllOnPage())}
                aria-label="全选本页"
              />
              <span className="text-xs font-medium text-muted-foreground">全选本页</span>
              <span className="ml-auto text-[11px] text-muted-foreground/70">
                本页 <span className="tabular-nums">{records.length}</span> 条 · 未读{" "}
                <span className="tabular-nums">{pageUnread}</span> 条
              </span>
              {virtualOn && (
                /* 虚拟模式低调标识: 提示当前走大列表性能渲染路径 */
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="inline-flex shrink-0 items-center rounded-full border bg-muted/40 px-2 py-0.5 text-[10px] font-medium text-muted-foreground">
                      虚拟渲染
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>大列表性能模式:仅渲染可视区行</TooltipContent>
                </Tooltip>
              )}
            </div>
            {virtualOn ? (
              /* 虚拟模式: ul 自身即滚动容器(限高); 行绝对定位偏移由 useVirtualizer 计算。
                 分隔线由行组件自带(父级不能再用 divide-y: ul 首子是总高占位 li, 且行脱离文档流) */
              <ul
                ref={scrollRef}
                className="relative max-h-[calc(100dvh-18rem)] overflow-y-auto scroll-area"
              >
                {/* 总高占位: 撑出真实滚动高度(空占位对读屏隐藏) */}
                <li aria-hidden style={{ height: rowVirtualizer.getTotalSize() }} />
                {rowVirtualizer.getVirtualItems().map((vRow) => {
                  const n = records[vRow.index]
                  if (!n) return null
                  return (
                    <NotificationRow
                      key={vRow.key}
                      n={n}
                      index={vRow.index}
                      isSelected={selected.has(n.id)}
                      isFocused={focusIndex === vRow.index}
                      compact={compact}
                      query={debouncedQ}
                      onOpen={openItem}
                      onToggle={toggleOne}
                      onMarkRead={markReadOne}
                      onDelete={removeOne}
                      removePending={removeMutation.isPending}
                      separated={vRow.index < records.length - 1}
                      virtualStart={vRow.start}
                      measureRef={rowVirtualizer.measureElement}
                    />
                  )
                })}
              </ul>
            ) : (
              /* 普通模式: 与改造前完全一致的全量渲染(无 max-h, 父级 divide-y 分隔, 行组件不带边线) */
              <ul ref={listRef} className="divide-y">
                {records.map((n, i) => (
                  <NotificationRow
                    key={n.id}
                    n={n}
                    index={i}
                    isSelected={selected.has(n.id)}
                    isFocused={focusIndex === i}
                    compact={compact}
                    query={debouncedQ}
                    onOpen={openItem}
                    onToggle={toggleOne}
                    onMarkRead={markReadOne}
                    onDelete={removeOne}
                    removePending={removeMutation.isPending}
                    separated={false}
                  />
                ))}
              </ul>
            )}
            {/* 键位提示条(桌面端显示, muted 不抢眼) */}
            <div className="hidden flex-wrap items-center gap-x-4 gap-y-1 border-t bg-muted/20 px-4 py-1.5 text-[11px] text-muted-foreground/80 md:flex sm:px-5">
              <span className="inline-flex items-center gap-1.5">
                <Kbd>J</Kbd>/<Kbd>K</Kbd> 移动
              </span>
              <span className="inline-flex items-center gap-1.5">
                <Kbd>Space</Kbd> 选择
              </span>
              <span className="inline-flex items-center gap-1.5">
                <Kbd>X</Kbd> 标记已读
              </span>
              <span className="inline-flex items-center gap-1.5">
                <Kbd>Esc</Kbd> 取消
              </span>
              {/* ? 帮助入口: 点击与按键均可打开 (Dialog 打开期间 hook 已禁用, 不会重复触发) */}
              <button
                type="button"
                onClick={() => setHelpOpen(true)}
                title="打开键盘快捷键帮助"
                aria-label="打开键盘快捷键帮助"
                className="ml-auto inline-flex items-center gap-1.5 rounded-sm outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
              >
                <Kbd>?</Kbd> 帮助
              </button>
            </div>
          </>
        )}
        {!listQuery.isLoading && !listQuery.isError && total > 0 && (
          <div className="border-t px-4 py-3">
            <DataPagination
              page={page}
              size={size}
              total={total}
              onPageChange={setPage}
              onSizeChange={setSize}
              sizeOptions={[10, 20, 50, 100, 200]}
            />
          </div>
        )}
      </div>

      {/* 批量删除二次确认 */}
      <ConfirmDialog
        open={batchDeleteOpen}
        onOpenChange={(o) => !o && setBatchDeleteOpen(false)}
        title={`删除选中的 ${selected.size} 条通知?`}
        description={
          <>
            将永久删除当前选中的 <b className="tabular-nums">{selected.size}</b> 条通知
            (含已读与未读),删除后不可恢复。
          </>
        }
        confirmText="批量删除"
        loading={removeBatchMutation.isPending}
        onConfirm={doBatchDelete}
      />

      {canManage && (
        <SendNotificationDialog open={sendOpen} onOpenChange={setSendOpen} onSent={invalidate} />
      )}

      <NotificationPrefsDialog open={prefsOpen} onOpenChange={setPrefsOpen} />

      {/* ? 键 / 提示条入口打开的快捷键帮助 */}
      <HotkeyHelpDialog open={helpOpen} onOpenChange={setHelpOpen} />
    </div>
  )
}

/* ============ 通知行(普通 / 虚拟两种渲染模式共用) ============ */

interface NotificationRowProps {
  n: NotificationRecord
  /** 行序号(0 起): data-notify-row 与虚拟模式 data-index 同源 */
  index: number
  isSelected: boolean
  isFocused: boolean
  compact: boolean
  /** 搜索命中高亮用的已 debounce 关键词 */
  query: string
  onOpen: (n: NotificationRecord) => void
  onToggle: (id: string) => void
  onMarkRead: (id: string) => void
  onDelete: (id: string) => void
  removePending: boolean
  /** 行自带底部分隔线: 虚拟模式 true(绝对定位行, 父级无 divide-y); 小列表 false(沿用父级 divide-y, 避免双 border) */
  separated: boolean
  /** 虚拟模式: 该行在滚动内容中的起始偏移(px); 普通模式 undefined */
  virtualStart?: number
  /** 虚拟模式: useVirtualizer.measureElement 转发(动态行高测量, 依据 data-index 关联缓存) */
  measureRef?: (node: HTMLLIElement | null) => void
}

/**
 * 单条通知行。普通列表与虚拟列表共用同一份 JSX(杜绝双份漂移); memo + 全原始值 /
 * 稳定引用 props: 焦点移动/选中切换时只有状态真正变化的行重渲染。
 * 虚拟模式由父级注入 virtualStart(绝对定位 translateY 偏移)与 measureRef(measureElement
 * 经 data-index 识别行); 两种模式下行内容与交互完全一致。
 */
const NotificationRow = React.memo(function NotificationRow({
  n,
  index,
  isSelected,
  isFocused,
  compact,
  query,
  onOpen,
  onToggle,
  onMarkRead,
  onDelete,
  removePending,
  separated,
  virtualStart,
  measureRef,
}: NotificationRowProps) {
  const meta = TYPE_META[n.type] ?? TYPE_META.INFO
  return (
    <li
      ref={measureRef}
      data-notify-row={index}
      data-focused={isFocused || undefined}
      data-index={virtualStart !== undefined ? index : undefined}
      style={
        virtualStart !== undefined
          ? {
              position: "absolute",
              top: 0,
              left: 0,
              width: "100%",
              transform: `translateY(${virtualStart}px)`,
            }
          : undefined
      }
      className={cn(
        "group relative flex items-start gap-3 px-4 transition-colors sm:px-5",
        compact ? "py-2" : "py-3.5",
        separated && "border-b border-border",
        // 焦点行:ring 高亮 + 背景微亮;选中行:bg + 复选框;两者可叠加且视觉可区分
        isFocused && "ring-1 ring-inset ring-primary/60",
        isSelected
          ? "bg-muted/50 hover:bg-muted/70"
          : isFocused
            ? "bg-primary/[0.05] hover:bg-primary/[0.08]"
            : n.isRead
              ? "hover:bg-accent/40"
              : "bg-primary/[0.035] hover:bg-primary/[0.07]"
      )}
    >
      {/* 未读标记(选中态下依然保留,与行背景不冲突) */}
      {!n.isRead && (
        <span aria-hidden className="absolute inset-y-0 left-0 w-[3px] bg-primary" />
      )}
      {/* 行复选框(已读行也可选,供批量删除) */}
      <Checkbox
        checked={isSelected}
        onCheckedChange={() => onToggle(n.id)}
        aria-label={`选择通知:${n.title}`}
        className="mt-1"
      />
      <span
        className={cn(
          "mt-0.5 flex shrink-0 items-center justify-center rounded-xl border",
          compact ? "size-8" : "size-9",
          meta.chip
        )}
      >
        <meta.icon className={compact ? "size-4" : "size-4.5"} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => onOpen(n)}
            className={cn(
              "truncate text-left transition-colors",
              compact ? "text-[13px]" : "text-sm",
              n.isRead
                ? "font-medium text-foreground/85 hover:text-foreground"
                : "font-semibold text-foreground hover:underline"
            )}
            title={n.title}
          >
            <Highlight text={n.title} query={query} />
          </button>
          <span
            className={cn(
              "inline-flex items-center gap-1 rounded border px-1.5 py-px text-[10px] font-medium",
              meta.chip
            )}
          >
            {meta.label}
          </span>
        </div>
        <p
          className={cn(
            "mt-1 line-clamp-2 leading-relaxed text-muted-foreground",
            compact ? "text-[11px]" : "text-xs"
          )}
        >
          <Highlight text={n.content || "—"} query={query} />
        </p>
        <div className="mt-1.5 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground/70">
          <span title={formatDateTime(n.createdAt)}>{formatRelative(n.createdAt)}</span>
          {n.link && (
            <button
              type="button"
              onClick={() => onOpen(n)}
              className="inline-flex items-center gap-0.5 text-primary/80 hover:text-primary hover:underline"
            >
              查看详情
              <ArrowRight className="size-3" />
            </button>
          )}
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-1 self-center opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
        {!n.isRead && (
          <Button
            variant="ghost"
            size="icon"
            className="size-8 text-muted-foreground"
            title="标记已读"
            aria-label="标记已读"
            onClick={() => onMarkRead(n.id)}
          >
            <CheckCheck className="size-4" />
          </Button>
        )}
        <Button
          variant="ghost"
          size="icon"
          className="size-8 text-muted-foreground hover:text-destructive"
          title="删除"
          aria-label="删除通知"
          onClick={() => onDelete(n.id)}
          disabled={removePending}
        >
          <Trash2 className="size-4" />
        </Button>
      </div>
    </li>
  )
})

/* ============ 键位提示 kbd ============ */

/**
 * 键帽组件: kbd 语义标签 + 语义色 + 底边内阴影模拟立体键帽, 深浅色均精致;
 * 固定行高与最小宽度, 单字符(如 ?)也能居中对齐。
 */
function Kbd({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <kbd
      className={cn(
        "inline-flex h-5 min-w-5 items-center justify-center rounded-md border bg-muted px-1.5 font-mono text-[10px] font-medium leading-none text-muted-foreground",
        "shadow-[inset_0_-2px_0_0_rgb(0_0_0/0.08)] dark:shadow-[inset_0_-2px_0_0_rgb(0_0_0/0.45)]",
        className
      )}
    >
      {children}
    </kbd>
  )
}

/* ============ 键盘快捷键帮助对话框 ============ */

/** 帮助条目: keys 为等价键位组合(如 J 与 ↓), desc 为右侧说明 */
interface HotkeyHelpItem {
  keys: string[]
  desc: string
}

/** 分组展示: 导航 / 选择 / 其他 */
const HOTKEY_HELP_GROUPS: { label: string; icon: LucideIcon; items: HotkeyHelpItem[] }[] = [
  {
    label: "导航",
    icon: MoveVertical,
    items: [
      { keys: ["J", "↓"], desc: "焦点移到下一行" },
      { keys: ["K", "↑"], desc: "焦点移到上一行" },
    ],
  },
  {
    label: "选择",
    icon: ListChecks,
    items: [
      { keys: ["Space"], desc: "选中 / 取消选中焦点行" },
      { keys: ["X"], desc: "将选中行批量标记已读（无选中时对焦点行单条已读）" },
    ],
  },
  {
    label: "其他",
    icon: CircleHelp,
    items: [
      { keys: ["Esc"], desc: "清空选择并复位焦点" },
      { keys: ["?"], desc: "打开本帮助（Shift + /）" },
    ],
  },
]

/** 快捷键帮助 Dialog: 分组键位 + 说明, 与底部 kbd 提示条及 ? 键联动 */
function HotkeyHelpDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto scroll-area sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Keyboard className="size-4.5 text-primary" />
            键盘快捷键
          </DialogTitle>
          <DialogDescription>通知中心支持以下键盘操作，提升批量处理效率</DialogDescription>
        </DialogHeader>

        {/* 分组键位列表(窄屏可滚动) */}
        <div className="space-y-4">
          {HOTKEY_HELP_GROUPS.map((group) => (
            <section key={group.label}>
              <p className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/80">
                <group.icon className="size-3.5 text-primary/70" aria-hidden />
                {group.label}
              </p>
              <ul className="mt-1.5 space-y-1">
                {group.items.map((item) => (
                  <li
                    key={item.desc}
                    className="flex items-center justify-between gap-4 rounded-lg bg-muted/40 px-3 py-1.5"
                  >
                    <span className="flex shrink-0 items-center gap-1">
                      {item.keys.map((k, i) => (
                        <React.Fragment key={k}>
                          {i > 0 && <span aria-hidden className="text-[10px] text-muted-foreground/60">/</span>}
                          <Kbd>{k}</Kbd>
                        </React.Fragment>
                      ))}
                    </span>
                    <span className="min-w-0 text-right text-xs leading-relaxed text-foreground/85">{item.desc}</span>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>

        <p className="border-t pt-3 text-[11px] leading-relaxed text-muted-foreground/75">
          快捷键仅在桌面端（≥768px）且无对话框遮挡时生效。
        </p>
      </DialogContent>
    </Dialog>
  )
}

/* ============ 通知偏好对话框 ============ */

/** 静音类型卡片选中态配色(复用 TYPE_META 四色) */
const MUTED_CARD_META: Record<NotificationType, { selected: string; badge: string }> = {
  INFO: {
    selected: "border-primary/45 bg-primary/10 text-primary",
    badge: "bg-primary text-primary-foreground",
  },
  SUCCESS: {
    selected: "border-emerald-500/45 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
    badge: "bg-emerald-500 text-white",
  },
  WARNING: {
    selected: "border-amber-500/50 bg-amber-500/10 text-amber-600 dark:text-amber-400",
    badge: "bg-amber-500 text-white",
  },
  SECURITY: {
    selected: "border-red-500/45 bg-red-500/10 text-red-600 dark:text-red-400",
    badge: "bg-red-500 text-white",
  },
}

const DEFAULT_PREFS: NotificationPrefs = {
  dndEnabled: false,
  dndStart: "22:00",
  dndEnd: "07:00",
  mutedTypes: [],
}

function NotificationPrefsDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (o: boolean) => void
}) {
  const queryClient = useQueryClient()
  const prefsQuery = useQuery({
    queryKey: ["notification-prefs"],
    queryFn: () => notificationsApi.getPrefs(),
    enabled: open,
    staleTime: 30_000,
    retry: 1,
  })

  /* 本地草稿:对话框打开且数据就绪时回填 */
  const [draft, setDraft] = React.useState<NotificationPrefs>(DEFAULT_PREFS)
  const [hydrated, setHydrated] = React.useState(false)
  React.useEffect(() => {
    if (open) {
      setHydrated(false)
      setDraft(DEFAULT_PREFS)
    }
  }, [open])
  React.useEffect(() => {
    if (open && prefsQuery.data && !hydrated) {
      setDraft({
        dndEnabled: !!prefsQuery.data.dndEnabled,
        dndStart: prefsQuery.data.dndStart || DEFAULT_PREFS.dndStart,
        dndEnd: prefsQuery.data.dndEnd || DEFAULT_PREFS.dndEnd,
        mutedTypes: Array.isArray(prefsQuery.data.mutedTypes) ? [...prefsQuery.data.mutedTypes] : [],
      })
      setHydrated(true)
    }
  }, [open, prefsQuery.data, hydrated])

  const dirty = React.useMemo(() => {
    const src = prefsQuery.data ?? DEFAULT_PREFS
    return (
      draft.dndEnabled !== !!src.dndEnabled ||
      draft.dndStart !== (src.dndStart || DEFAULT_PREFS.dndStart) ||
      draft.dndEnd !== (src.dndEnd || DEFAULT_PREFS.dndEnd) ||
      draft.mutedTypes.length !== (src.mutedTypes?.length ?? 0) ||
      draft.mutedTypes.some((t) => !src.mutedTypes?.includes(t))
    )
  }, [draft, prefsQuery.data])

  const saveMutation = useMutation({
    mutationFn: () =>
      notificationsApi.savePrefs({
        dndEnabled: draft.dndEnabled,
        dndStart: draft.dndStart,
        dndEnd: draft.dndEnd,
        mutedTypes: draft.mutedTypes,
      }),
    onSuccess: () => {
      toast.success("通知偏好已保存")
      /* 即时生效:socket 钩子与铃铛共享该缓存 */
      void queryClient.invalidateQueries({ queryKey: ["notification-prefs"] })
      onOpenChange(false)
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "保存失败"),
  })

  const toggleMuted = (t: NotificationType) => {
    setDraft((d) => ({
      ...d,
      mutedTypes: d.mutedTypes.includes(t)
        ? d.mutedTypes.filter((x) => x !== t)
        : [...d.mutedTypes, t],
    }))
  }

  /* 跨午夜时段提示:开始 > 结束时为 [start,24:00) ∪ [00:00,end] */
  const crossMidnight = draft.dndStart > draft.dndEnd

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto scroll-area sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Settings2 className="size-5 text-primary" />
            通知偏好
          </DialogTitle>
          <DialogDescription>免打扰时段与类型静音,仅影响桌面提醒,角标与列表照常实时更新</DialogDescription>
        </DialogHeader>

        {prefsQuery.isLoading ? (
          <div className="space-y-4 py-2">
            <Skeleton className="h-16 w-full rounded-xl" />
            <Skeleton className="h-24 w-full rounded-xl" />
            <div className="grid grid-cols-4 gap-2">
              {Array.from({ length: 4 }).map((_, i) => (
                <Skeleton key={i} className="h-20 rounded-xl" />
              ))}
            </div>
          </div>
        ) : prefsQuery.isError ? (
          <ErrorState
            className="border-0 bg-transparent"
            message={prefsQuery.error instanceof Error ? prefsQuery.error.message : undefined}
            onRetry={() => prefsQuery.refetch()}
          />
        ) : (
          <div className="space-y-5">
            {/* 免打扰开关 */}
            <div
              className={cn(
                "rounded-xl border p-4 transition-colors",
                draft.dndEnabled ? "border-amber-500/40 bg-amber-500/[0.06]" : "bg-muted/30"
              )}
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <p className="flex items-center gap-2 text-sm font-semibold">
                    <Moon className="size-4 text-amber-500" />
                    免打扰时段
                  </p>
                  <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                    开启后,在设定时段内不再弹出通知提醒,角标与列表照常更新
                  </p>
                </div>
                <Switch
                  checked={draft.dndEnabled}
                  onCheckedChange={(v) => setDraft((d) => ({ ...d, dndEnabled: v }))}
                  aria-label="开启免打扰"
                  className="data-[state=checked]:bg-amber-500"
                />
              </div>

              {/* 时段选择:DND 关闭时禁用 */}
              <div
                className={cn(
                  "mt-3 grid grid-cols-[1fr_auto_1fr] items-center gap-2 transition-opacity duration-300",
                  !draft.dndEnabled && "pointer-events-none opacity-50"
                )}
              >
                <div className="space-y-1.5">
                  <Label htmlFor="dnd-start" className="text-[11px] text-muted-foreground">
                    开始
                  </Label>
                  <Input
                    id="dnd-start"
                    type="time"
                    step={600}
                    value={draft.dndStart}
                    onChange={(e) => setDraft((d) => ({ ...d, dndStart: e.target.value || "22:00" }))}
                    disabled={!draft.dndEnabled}
                    aria-label="免打扰开始时间"
                    className="[color-scheme:light] font-mono tabular-nums dark:[color-scheme:dark]"
                  />
                </div>
                <ArrowRight className="mt-5 size-4 shrink-0 text-muted-foreground/60" aria-hidden />
                <div className="space-y-1.5">
                  <Label htmlFor="dnd-end" className="text-[11px] text-muted-foreground">
                    结束
                  </Label>
                  <Input
                    id="dnd-end"
                    type="time"
                    step={600}
                    value={draft.dndEnd}
                    onChange={(e) => setDraft((d) => ({ ...d, dndEnd: e.target.value || "07:00" }))}
                    disabled={!draft.dndEnabled}
                    aria-label="免打扰结束时间"
                    className="[color-scheme:light] font-mono tabular-nums dark:[color-scheme:dark]"
                  />
                </div>
              </div>
              {draft.dndEnabled && (
                <p className="mt-2.5 flex items-center gap-1.5 text-[11px] text-muted-foreground">
                  <span className="inline-block size-1.5 rounded-full bg-amber-500" aria-hidden />
                  {crossMidnight
                    ? `生效区间 ${draft.dndStart} → 次日 ${draft.dndEnd}(跨午夜时段)`
                    : `生效区间 ${draft.dndStart} → ${draft.dndEnd} 当日`}
                </p>
              )}
            </div>

            {/* 静音类型 */}
            <div className="space-y-2.5">
              <div>
                <p className="text-sm font-semibold">静音类型</p>
                <p className="mt-0.5 text-xs text-muted-foreground">静音后该类型通知不弹提醒,仅静默入列</p>
              </div>
              <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                {(Object.keys(TYPE_META) as NotificationType[]).map((t) => {
                  const meta = TYPE_META[t]
                  const muted = draft.mutedTypes.includes(t)
                  const card = MUTED_CARD_META[t]
                  return (
                    <button
                      key={t}
                      type="button"
                      role="checkbox"
                      aria-checked={muted}
                      onClick={() => toggleMuted(t)}
                      title={muted ? `已静音${meta.label}通知,点击取消` : `点击静音${meta.label}通知`}
                      className={cn(
                        "relative flex flex-col items-center gap-1.5 rounded-xl border px-2 py-3 text-center outline-none transition-all focus-visible:ring-2 focus-visible:ring-ring",
                        muted
                          ? card.selected
                          : "border-border bg-background text-muted-foreground hover:border-muted-foreground/30 hover:bg-accent"
                      )}
                    >
                      {/* 左上角勾选角标 */}
                      {muted && (
                        <span
                          className={cn(
                            "absolute -left-1.5 -top-1.5 flex size-5 items-center justify-center rounded-full shadow-sm",
                            card.badge
                          )}
                        >
                          <Check className="size-3" />
                        </span>
                      )}
                      <meta.icon className="size-5" />
                      <span className="text-xs font-medium">{meta.label}</span>
                      <span className="text-[10px] leading-none text-muted-foreground/70">
                        {muted ? "已静音" : "提醒中"}
                      </span>
                    </button>
                  )
                })}
              </div>
            </div>
          </div>
        )}

        <DialogFooter className="gap-2 sm:gap-0">
          <Button
            variant="ghost"
            size="sm"
            className="mr-auto text-muted-foreground"
            onClick={() => setDraft({ ...DEFAULT_PREFS, mutedTypes: [] })}
            disabled={!hydrated || saveMutation.isPending}
          >
            <RotateCcw className="size-3.5" />
            恢复默认
          </Button>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button onClick={() => saveMutation.mutate()} disabled={!dirty || saveMutation.isPending}>
            {saveMutation.isPending ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Check className="size-4" />
            )}
            保存更改
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/* ============ 发送通知对话框 ============ */

type TargetKind = "USER" | "ORG" | "TENANT"

const TARGET_OPTIONS: { v: TargetKind; label: string; icon: typeof User }[] = [
  { v: "USER", label: "单个用户", icon: User },
  { v: "ORG", label: "组织成员", icon: Building2 },
  { v: "TENANT", label: "租户全员", icon: Landmark },
]

function SendNotificationDialog({
  open,
  onOpenChange,
  onSent,
}: {
  open: boolean
  onOpenChange: (o: boolean) => void
  onSent: () => void
}) {
  const [kind, setKind] = React.useState<TargetKind>("USER")
  const [userId, setUserId] = React.useState("")
  const [orgId, setOrgId] = React.useState<string | null>(null)
  const [tenantId, setTenantId] = React.useState<string | null>(null)
  const [title, setTitle] = React.useState("")
  const [content, setContent] = React.useState("")
  const [type, setType] = React.useState<NotificationType>("INFO")
  const [link, setLink] = React.useState("")

  const currentTenantId = useAuthStore((s) => s.profile?.tenant.id) ?? null

  const usersQuery = useQuery({
    queryKey: ["send-notify-users", currentTenantId],
    queryFn: async () =>
      asPage<{ id: string; username: string; realName: string }>(
        await usersApi.list({ page: 1, size: 100, tenantId: currentTenantId ?? undefined })
      ),
    enabled: open && kind === "USER",
    staleTime: 60_000,
    retry: 1,
  })
  const orgTreeQuery = useQuery({
    queryKey: ["send-notify-orgs", currentTenantId],
    queryFn: async () =>
      asArray<PickerNode>(await organizationsApi.tree(currentTenantId ?? "")),
    enabled: open && kind === "ORG" && !!currentTenantId,
    staleTime: 60_000,
    retry: 1,
  })
  const tenantsQuery = useQuery({
    queryKey: ["send-notify-tenants"],
    queryFn: async () =>
      asPage<{ id: string; name: string }>(await tenantsApi.list({ page: 1, size: 50 })),
    enabled: open && kind === "TENANT",
    staleTime: 60_000,
    retry: 1,
  })

  const sendMutation = useMutation({
    mutationFn: () =>
      notificationsApi.send({
        title: title.trim(),
        content: content.trim(),
        type,
        link: link.trim() || undefined,
        userId: kind === "USER" ? userId : undefined,
        orgId: kind === "ORG" ? (orgId ?? undefined) : undefined,
        tenantId: kind === "TENANT" ? (tenantId ?? undefined) : undefined,
      }),
    onSuccess: (d) => {
      toast.success(`通知已发送给 ${d.sent} 位用户`)
      onOpenChange(false)
      onSent()
      setTitle("")
      setContent("")
      setLink("")
      setUserId("")
      setOrgId(null)
      setTenantId(null)
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "发送失败"),
  })

  const targetReady =
    (kind === "USER" && !!userId) ||
    (kind === "ORG" && !!orgId) ||
    (kind === "TENANT" && !!tenantId)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto scroll-area sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Megaphone className="size-5 text-primary" />
            发送通知
          </DialogTitle>
          <DialogDescription>向指定用户、组织或租户的全体成员发送站内通知</DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {/* 目标范围 */}
          <div className="space-y-2">
            <Label>目标范围</Label>
            <RadioGroup
              value={kind}
              onValueChange={(v) => setKind(v as TargetKind)}
              className="grid grid-cols-3 gap-2"
            >
              {TARGET_OPTIONS.map((o) => (
                <Label
                  key={o.v}
                  className={cn(
                    "flex cursor-pointer flex-col items-center gap-1.5 rounded-xl border p-3 text-xs font-medium transition-colors",
                    kind === o.v
                      ? "border-primary/50 bg-primary/8 text-primary"
                      : "text-muted-foreground hover:bg-accent"
                  )}
                >
                  <RadioGroupItem value={o.v} className="sr-only" />
                  <o.icon className="size-4.5" />
                  {o.label}
                </Label>
              ))}
            </RadioGroup>
          </div>

          {kind === "USER" && (
            <div className="space-y-2">
              <Label>选择用户</Label>
              <Select value={userId} onValueChange={setUserId}>
                <SelectTrigger aria-label="选择用户">
                  <SelectValue placeholder="选择接收用户" />
                </SelectTrigger>
                <SelectContent className="max-h-64">
                  {usersQuery.isLoading ? (
                    <div className="flex items-center justify-center gap-2 py-6 text-xs text-muted-foreground">
                      <Loader2 className="size-4 animate-spin" />
                      加载用户中...
                    </div>
                  ) : (
                    usersQuery.data?.records.map((u) => (
                      <SelectItem key={u.id} value={u.id}>
                        {u.realName || u.username}({u.username})
                      </SelectItem>
                    ))
                  )}
                </SelectContent>
              </Select>
            </div>
          )}
          {kind === "ORG" && (
            <div className="space-y-2">
              <Label>选择组织(广播给全部成员)</Label>
              <EntityTreePicker
                nodes={orgTreeQuery.data ?? []}
                value={orgId}
                onSelect={(id) => setOrgId(id)}
                placeholder="选择目标组织"
              />
            </div>
          )}
          {kind === "TENANT" && (
            <div className="space-y-2">
              <Label>选择租户(广播给全部成员)</Label>
              <Select value={tenantId ?? ""} onValueChange={setTenantId}>
                <SelectTrigger aria-label="选择租户">
                  <SelectValue placeholder="选择目标租户" />
                </SelectTrigger>
                <SelectContent className="max-h-64">
                  {tenantsQuery.data?.records.map((t) => (
                    <SelectItem key={t.id} value={t.id}>
                      {t.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}

          {/* 类型 + 链接 */}
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-2">
              <Label>通知类型</Label>
              <Select value={type} onValueChange={(v) => setType(v as NotificationType)}>
                <SelectTrigger aria-label="通知类型">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {(Object.keys(TYPE_META) as NotificationType[]).map((t) => {
                    const Ico = TYPE_META[t].icon
                    return (
                      <SelectItem key={t} value={t}>
                        <span className="inline-flex items-center gap-2">
                          <Ico className="size-3.5" />
                          {TYPE_META[t].label}
                        </span>
                      </SelectItem>
                    )
                  })}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label>
                关联链接<span className="text-muted-foreground/60">(可选)</span>
              </Label>
              <Input
                value={link}
                onChange={(e) => setLink(e.target.value)}
                placeholder="如 /system/audit-logs"
                className="font-mono text-xs"
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label>标题</Label>
            <Input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="一句话说明通知主题"
              maxLength={200}
            />
          </div>
          <div className="space-y-2">
            <Label>内容</Label>
            <Textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              placeholder="通知正文,支持换行"
              rows={4}
              className="resize-none"
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button
            onClick={() => sendMutation.mutate()}
            disabled={sendMutation.isPending || !title.trim() || !targetReady}
          >
            {sendMutation.isPending ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Send className="size-4" />
            )}
            发送
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
