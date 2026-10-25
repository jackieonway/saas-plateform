import * as React from "react"
import { Navigate, Outlet, useLocation } from "react-router-dom"
import { AnimatePresence, motion } from "framer-motion"
import { Loader2 } from "lucide-react"
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Button } from "@/components/ui/button"
import { TooltipProvider } from "@/components/ui/tooltip"
import { AppIcon } from "@/lib/icons"
import { AppRail } from "@/components/layout/AppRail"
import { AppSidebar } from "@/components/layout/AppSidebar"
import { TopBar } from "@/components/layout/TopBar"
import { RouteRecoveryBoundary } from "@/components/RouteRecoveryBoundary"
import { RoutePermissionGuard } from "@/components/layout/RoutePermissionGuard"
import { ErrorState } from "@/components/states"
import { useAuthStore } from "@/stores/auth"
import { BrandMark } from "@/components/brand"
import { usePlatformBranding } from "@/hooks/use-platform-branding"
import { useMenusQuery, useProfileQuery } from "@/hooks/use-auth-data"
import { useDocumentTitle, pageTitleFor } from "@/hooks/use-document-title"
import { pushRecentVisit } from "@/hooks/use-recent-visits"
import { useApplicationNavigation } from "@/hooks/use-application-navigation"
import { invalidateApplicationSelection } from "@/lib/application-navigation"
import type { MenuNode } from "@/api/types"

const CommandPalette = React.lazy(() => import("@/components/layout/CommandPalette").then(module => ({ default: module.CommandPalette })))

export default function AppLayout() {
  const { platformName } = usePlatformBranding()
  const token = useAuthStore((s) => s.token)
  const contextEpoch = useAuthStore((s) => s.contextEpoch)
  const contextKey = useAuthStore((s) => s.contextKey)
  const contextSwitching = useAuthStore((s) => s.contextSwitching)
  const profile = useAuthStore((s) => s.profile)
  const setProfile = useAuthStore((s) => s.setProfile)
  const currentApplicationId = useAuthStore((s) => s.currentApplicationId)
  const setCurrentApplication = useAuthStore((s) => s.setCurrentApplication)
  const ensureApplication = useAuthStore((s) => s.ensureApplication)
  const { pathname, search, key: locationKey } = useLocation()
  const isMicroRoute = /^\/micro\/[^/]+(?:\/|$)/.test(pathname)
  const { selectApplication, access } = useApplicationNavigation()
  React.useLayoutEffect(() => {
    invalidateApplicationSelection()
    return () => { invalidateApplicationSelection() }
  }, [locationKey])

  // 路由联动 document.title(/embed 标题随菜单外链标题动态变化, 由 EmbedPage 自行管理)
  useDocumentTitle(pathname === "/embed" ? null : pageTitleFor(pathname))

  const profileQuery = useProfileQuery()

  // 同步 profile 到 store
  React.useEffect(() => {
    if (profileQuery.data) setProfile(profileQuery.data, contextEpoch)
  }, [profileQuery.data, setProfile, contextEpoch])

  // 应用可用列表变化时校正当前应用
  React.useEffect(() => {
    if (profile?.applications) ensureApplication(profile.applications)
  }, [profile?.applications, ensureApplication])

  // 深链路由 → 自动切换到该路由所属应用(若已授权),保证侧栏与页面上下文一致
  // 相同路由/授权所属应用的轮询不覆盖手动选择; 授权所属应用变化后重新校正深链。
  const inferredPathRef = React.useRef<string | null>(null)
  React.useEffect(() => {
    const match = profile?.applications.find(
      (a) =>
        a.basePath &&
        a.basePath !== "/" &&
        (pathname === a.basePath || pathname.startsWith(`${a.basePath}/`))
    )
    const routeContext = `${contextEpoch}:${pathname}:${match?.id ?? ""}`
    if (inferredPathRef.current === routeContext) return
    inferredPathRef.current = routeContext
    if (match && match.id !== currentApplicationId) setCurrentApplication(match.id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pathname, profile?.applications, contextEpoch])

  const handleSelectApp = React.useCallback(
    (appId: string) => { void selectApplication(appId, pathname) },
    [selectApplication, pathname]
  )

  // 路由切换回到顶部
  React.useEffect(() => {
    document.getElementById("app-scroll")?.scrollTo({ top: 0 })
  }, [pathname])

  /*
   * 页面过渡动画的 remount 粒度:
   * 普通页面每个 pathname 一档(逐页转场动画);
   * 微前端子应用(/micro/:appCode/*)按应用一档 —— 子路径变化只触发容器内的
   * lifecycle.update 增量推送(计时器/输入等子应用状态保留), 不重挂整棵路由树。
   * (若用 pathname 作 key, 任何子路径变化都会把 MicroMountZone 连根重挂,
   *  update 契约将被布局层击穿 —— v1 全量重挂的真正机制即在此。)
   */
  const pageTransitionKey = React.useMemo(() => {
    if (pathname === "/micro" || pathname.startsWith("/micro/")) {
      const seg = pathname.split("/") // ['', 'micro', appCode, ...]
      return `${contextEpoch}:/${seg[1]}/${seg[2] ?? ""}`
    }
    return `${contextEpoch}:${pathname}`
  }, [pathname, contextEpoch])


  const [sidebarOpen, setSidebarOpen] = React.useState(() => {
    try {
      return localStorage.getItem("platform-sidebar") !== "collapsed"
    } catch {
      return true
    }
  })
  React.useEffect(() => {
    try {
      localStorage.setItem("platform-sidebar", sidebarOpen ? "open" : "collapsed")
    } catch {
      /* ignore */
    }
  }, [sidebarOpen])

  const [mobileNavOpen, setMobileNavOpen] = React.useState(false)
  const [paletteOpen, setPaletteOpen] = React.useState(false)
  React.useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault()
        setPaletteOpen(true)
      }
    }
    window.addEventListener("keydown", handler)
    return () => window.removeEventListener("keydown", handler)
  }, [])

  const app = profile?.applications.find((a) => a.id === currentApplicationId) ?? null
  const menusQuery = useMenusQuery(currentApplicationId)
  const canRecordCurrentPath = access(pathname).status === "allowed"

  /* ---------- 最近访问记录(localStorage,个人工作台展示) ---------- */
  const recordedPathRef = React.useRef<string | null>(null)
  React.useEffect(() => {
    // 登录 / 设备 / 同意页不在 AppLayout 内,此为兜底防御;
    // /embed 为外链嵌入页, 不写入最近访问(避免高频访问出现长 URL 条目)
    if (["/login", "/device", "/consent", "/embed"].some((p) => pathname === p || pathname.startsWith(`${p}/`))) {
      return
    }
    if (recordedPathRef.current === pathname) return
    // 菜单仍在加载时先等一下,尽量带上菜单图标再记录
    if (menusQuery.isLoading || !canRecordCurrentPath) return

    const findIcon = (nodes: MenuNode[]): string | null => {
      for (const n of nodes) {
        if (n.path && n.path === pathname) return n.icon || null
        if (n.children?.length) {
          const hit = findIcon(n.children)
          if (hit) return hit
        }
      }
      return null
    }
    const FALLBACK_ICONS: Record<string, string> = {
      "/dashboard": "LayoutDashboard",
      "/profile": "User",
    }
    const icon =
      findIcon(menusQuery.data ?? []) ??
      FALLBACK_ICONS[pathname] ??
      null
    recordedPathRef.current = pathname
    pushRecentVisit({
      path: pathname,
      title: pageTitleFor(pathname),
      icon: icon ?? "Page",
    })
  }, [pathname, menusQuery.isLoading, menusQuery.data, canRecordCurrentPath])

  /* ---------- 守卫 ---------- */
  if (!token) {
    return <Navigate to={`/login?return=${encodeURIComponent(pathname + search)}`} replace />
  }

  /* ---------- 会话加载 ---------- */
  if (contextSwitching || (!profile && !profileQuery.isSuccess && !profileQuery.isError)) {
    return (
      <div className="flex min-h-screen flex-col items-center justify-center gap-4 bg-background">
        <span className="flex size-14 items-center justify-center rounded-2xl bg-primary/10 text-primary">
          <BrandMark className="size-8" />
        </span>
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" />
          {contextSwitching ? "正在切换组织..." : "正在恢复会话..."}
        </div>
      </div>
    )
  }

  if (profileQuery.isError && !profile) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background p-6">
        <div className="w-full max-w-md space-y-4">
          <ErrorState
            message={profileQuery.error instanceof Error ? profileQuery.error.message : undefined}
            onRetry={() => profileQuery.refetch()}
          />
          <div className="flex justify-center">
            <Button
              variant="ghost"
              size="sm"
              onClick={async () => {
                if (await useAuthStore.getState().logout()) window.location.assign("/login")
              }}
            >
              返回登录
            </Button>
          </div>
        </div>
      </div>
    )
  }

  const userDisplayName = profile?.user.realName || profile?.user.username || ""
  const navigationScope = profile && app && contextKey
    ? [contextKey, app.id].map(encodeURIComponent).join(":")
    : undefined

  const sidebar = (
    <AppSidebar
      app={app}
      menus={menusQuery.data ?? []}
      isLoading={menusQuery.isLoading}
      isError={menusQuery.isError}
      onRetry={() => menusQuery.refetch()}
      onCollapse={() => setSidebarOpen(false)}
      navigationScope={navigationScope}
      userDisplayName={userDisplayName}
    />
  )

  return (
    <TooltipProvider delayDuration={200}>
      <div className="flex h-dvh w-full overflow-hidden bg-background">
        {/* 桌面端导航 */}
        <div className="hidden lg:flex">
          <AppRail
            apps={profile?.applications ?? []}
            currentApplicationId={currentApplicationId}
            onSelect={handleSelectApp}
            sidebarOpen={sidebarOpen}
            onToggleSidebar={() => setSidebarOpen((o) => !o)}
          />
          {sidebarOpen && sidebar}
        </div>

        {/* 移动端抽屉 */}
        <Sheet open={mobileNavOpen} onOpenChange={setMobileNavOpen}>
          <SheetContent side="left" className="w-[280px] gap-0 bg-sidebar p-0 text-sidebar-foreground">
            <SheetHeader className="border-b border-sidebar-border px-4 py-3">
              <SheetTitle className="flex items-center gap-2.5 text-sidebar-foreground">
                <span className="flex size-8 items-center justify-center rounded-lg bg-sidebar-primary/15 text-sidebar-primary">
                  <BrandMark className="size-5" />
                </span>
                <span className="truncate" title={platformName}>{platformName}</span>
              </SheetTitle>
            </SheetHeader>
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
              <div className="flex gap-2 overflow-x-auto scroll-area border-b border-sidebar-border p-3">
                {(profile?.applications ?? []).map((a) => (
                  <button
                    key={a.id}
                    type="button"
                    aria-label={`切换到应用 ${a.name}`}
                    onClick={() => {
                      handleSelectApp(a.id)
                      setMobileNavOpen(false)
                    }}
                    className={`flex size-11 shrink-0 items-center justify-center rounded-xl transition-colors ${
                      a.id === currentApplicationId
                        ? "bg-sidebar-primary/20 text-sidebar-primary"
                        : "bg-sidebar-accent/60 text-sidebar-foreground/70"
                    }`}
                  >
                    <AppIcon name={a.icon} size={22} />
                  </button>
                ))}
              </div>
              <div className="min-h-0 flex-1 [&_.text-muted-foreground]:text-sidebar-foreground/60">
                <AppSidebar
                  app={app}
                  menus={menusQuery.data ?? []}
                  isLoading={menusQuery.isLoading}
                  isError={menusQuery.isError}
                  onRetry={() => menusQuery.refetch()}
                  navigationScope={navigationScope}
                  userDisplayName={userDisplayName}
                  className="w-full border-r-0"
                />
              </div>
            </div>
          </SheetContent>
        </Sheet>

        {/* 主体 */}
        <div className="flex min-w-0 flex-1 flex-col">
          <TopBar
            onOpenMobileNav={() => setMobileNavOpen(true)}
            onOpenCommandPalette={() => setPaletteOpen(true)}
            menuCollapsed={!sidebarOpen}
            onExpandMenu={() => setSidebarOpen(true)}
          />
          <main
            id="app-scroll"
            className={`flex-1 overflow-y-auto scroll-area ${isMicroRoute ? "p-0" : "p-4 md:p-6"}`}
            aria-live="polite"
          >
            <RouteRecoveryBoundary key={pageTransitionKey}>
            <RoutePermissionGuard>
              <AnimatePresence mode="wait">
                <motion.div
                  key={pageTransitionKey}
                  initial={{ opacity: 0, y: 8 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.15, ease: "easeOut" }}
                  className={isMicroRoute ? "min-h-full w-full" : "mx-auto w-full max-w-[1600px]"}
                >
                  <Outlet />
                </motion.div>
              </AnimatePresence>
            </RoutePermissionGuard>
            </RouteRecoveryBoundary>
          </main>
        </div>

        {paletteOpen && <div className="fixed bottom-4 right-4 z-50 max-w-lg">
          <RouteRecoveryBoundary loading={<span role="status" className="rounded-md border bg-card px-3 py-2 text-sm">命令面板加载中…</span>}>
            <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
          </RouteRecoveryBoundary>
          <button type="button" className="sr-only focus:not-sr-only" onClick={() => setPaletteOpen(false)}>关闭命令面板</button>
        </div>}
      </div>
    </TooltipProvider>
  )
}
