import * as React from "react"
import { Navigate, Outlet, useLocation, useNavigate } from "react-router-dom"
import { AnimatePresence, motion } from "framer-motion"
import { Loader2 } from "lucide-react"
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Button } from "@/components/ui/button"
import { TooltipProvider } from "@/components/ui/tooltip"
import { AppIcon } from "@/lib/icons"
import { AppRail } from "@/components/layout/AppRail"
import { AppSidebar } from "@/components/layout/AppSidebar"
import { TopBar } from "@/components/layout/TopBar"
import { CommandPalette } from "@/components/layout/CommandPalette"
import { ErrorState } from "@/components/states"
import { useAuthStore } from "@/stores/auth"
import { useQueryClient } from "@tanstack/react-query"
import { useMenusQuery, useProfileQuery } from "@/hooks/use-auth-data"
import { useDocumentTitle, pageTitleFor } from "@/hooks/use-document-title"
import { pushRecentVisit } from "@/hooks/use-recent-visits"
import { ROUTE_TITLES } from "@/lib/routes"
import { authApi } from "@/api/auth"
import { asArray } from "@/lib/api"
import { sortMenuTree } from "@/lib/tree"
import type { MenuNode } from "@/api/types"

function BrandMark({ className }: { className?: string }) {
  return (
    <span className={className} aria-hidden>
      <svg viewBox="0 0 48 48" className="size-full">
        <defs>
          <linearGradient id="brand-g" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="currentColor" stopOpacity="0.95" />
            <stop offset="1" stopColor="currentColor" stopOpacity="0.55" />
          </linearGradient>
        </defs>
        <path
          d="M12 30a7 7 0 0 1 .5-13.96A9.5 9.5 0 0 1 30.6 14a8 8 0 0 1 7.9 9.4A6 6 0 0 1 37 31H12Z"
          fill="url(#brand-g)"
          opacity="0.9"
        />
        <path
          d="M24 26l4.2 4.2-1.7 1.7-1.5-1.5v6.1h-2v-6.1l-1.5 1.5-1.7-1.7L24 26Z"
          fill="currentColor"
        />
        <path
          d="M18.2 33.8a6.4 6.4 0 0 1-5.7-3.5"
          stroke="currentColor"
          strokeWidth="2.2"
          strokeLinecap="round"
          fill="none"
        />
      </svg>
    </span>
  )
}

export default function AppLayout() {
  const token = useAuthStore((s) => s.token)
  const profile = useAuthStore((s) => s.profile)
  const setProfile = useAuthStore((s) => s.setProfile)
  const currentApplicationId = useAuthStore((s) => s.currentApplicationId)
  const setCurrentApplication = useAuthStore((s) => s.setCurrentApplication)
  const ensureApplication = useAuthStore((s) => s.ensureApplication)
  const { pathname, search } = useLocation()
  const navigate = useNavigate()

  // 路由联动 document.title(/embed 标题随菜单外链标题动态变化, 由 EmbedPage 自行管理)
  useDocumentTitle(pathname === "/embed" ? null : pageTitleFor(pathname))

  const profileQuery = useProfileQuery()
  const queryClient = useQueryClient()

  // 同步 profile 到 store
  React.useEffect(() => {
    if (profileQuery.data) setProfile(profileQuery.data)
  }, [profileQuery.data, setProfile])

  // 应用可用列表变化时校正当前应用
  React.useEffect(() => {
    if (profile?.applications) ensureApplication(profile.applications)
  }, [profile?.applications, ensureApplication])

  // 深链路由 → 自动切换到该路由所属应用(若已授权),保证侧栏与页面上下文一致
  // 仅在 pathname 变化时触发,避免与用户手动切换应用(不改路由)相互覆盖
  const inferredPathRef = React.useRef<string | null>(null)
  React.useEffect(() => {
    if (!profile?.applications?.length) return
    if (inferredPathRef.current === pathname) return
    inferredPathRef.current = pathname
    const match = profile.applications.find(
      (a) =>
        a.basePath &&
        a.basePath !== "/" &&
        (pathname === a.basePath || pathname.startsWith(`${a.basePath}/`))
    )
    if (match && match.id !== currentApplicationId) setCurrentApplication(match.id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pathname, profile?.applications])

  /** 切换应用:若当前页面不属于新应用,则跳转到该应用首个可导航菜单 */
  const handleSelectApp = React.useCallback(
    (appId: string) => {
      setCurrentApplication(appId)
      const app = (profile?.applications ?? []).find((a) => a.id === appId)
      if (!app?.basePath || app.basePath === "/") return
      if (pathname === app.basePath || pathname.startsWith(`${app.basePath}/`)) return
      void (async () => {
        try {
          const menus = await queryClient.fetchQuery({
            queryKey: ["auth-menus", appId],
            queryFn: async () =>
              sortMenuTree(asArray<MenuNode>(await authApi.menus(appId), "menus")),
            staleTime: 30_000,
          })
          const firstPath = (function walk(nodes: MenuNode[]): string | null {
            for (const n of nodes) {
              // 外链菜单无对应路由(BLANK 新窗口/SELF 走 /embed), 切换应用时跳过
              if (n.type !== "BUTTON" && n.path && !n.linkUrl) return n.path
              if (n.children?.length) {
                const found = walk(n.children)
                if (found) return found
              }
            }
            return null
          })(menus)
          if (firstPath) navigate(firstPath)
        } catch {
          /* 菜单拉取失败时仅切换侧栏,不跳转 */
        }
      })()
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [profile?.applications, pathname, setCurrentApplication, queryClient, navigate]
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
      return `/${seg[1]}/${seg[2] ?? ""}`
    }
    return pathname
  }, [pathname])


  const [sidebarOpen, setSidebarOpen] = React.useState(() => {
    try {
      return localStorage.getItem("dante-sidebar") !== "collapsed"
    } catch {
      return true
    }
  })
  React.useEffect(() => {
    try {
      localStorage.setItem("dante-sidebar", sidebarOpen ? "open" : "collapsed")
    } catch {
      /* ignore */
    }
  }, [sidebarOpen])

  const [mobileNavOpen, setMobileNavOpen] = React.useState(false)
  const [paletteOpen, setPaletteOpen] = React.useState(false)

  const app = profile?.applications.find((a) => a.id === currentApplicationId) ?? null
  const menusQuery = useMenusQuery(currentApplicationId)

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
    if (menusQuery.isLoading) return

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
    // 只记录真实存在的页面: 已知路由表命中或当前应用菜单命中
    // (避免 404 兜底页与 / 根重定向混入「最近访问」)
    const known = ROUTE_TITLES[pathname] !== undefined || icon !== null
    if (!known) {
      recordedPathRef.current = pathname
      return
    }
    recordedPathRef.current = pathname
    pushRecentVisit({
      path: pathname,
      title: pageTitleFor(pathname),
      icon: icon ?? "Page",
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pathname, menusQuery.isLoading, menusQuery.data])

  /* ---------- 守卫 ---------- */
  if (!token) {
    return <Navigate to={`/login?return=${encodeURIComponent(pathname + search)}`} replace />
  }

  /* ---------- 会话加载 ---------- */
  if (!profile && profileQuery.isLoading) {
    return (
      <div className="flex min-h-screen flex-col items-center justify-center gap-4 bg-background">
        <span className="flex size-14 items-center justify-center rounded-2xl bg-primary/10 text-primary">
          <BrandMark className="size-8" />
        </span>
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" />
          正在恢复会话...
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
              onClick={() => {
                useAuthStore.getState().logout()
                window.location.assign("/login")
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

  const sidebar = (
    <AppSidebar
      app={app}
      menus={menusQuery.data ?? []}
      isLoading={menusQuery.isLoading}
      isError={menusQuery.isError}
      onRetry={() => menusQuery.refetch()}
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
                <span className="flex size-8 items-center justify-center rounded-lg bg-primary/15 text-primary">
                  <BrandMark className="size-5" />
                </span>
                Dante Cloud
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
                        ? "bg-primary/20 text-primary"
                        : "bg-sidebar-accent/60 text-sidebar-foreground/70"
                    }`}
                  >
                    <AppIcon name={a.icon} size={22} />
                  </button>
                ))}
              </div>
              <div className="min-h-0 flex-1 [&_button]:!text-sidebar-foreground [&_.text-muted-foreground]:text-sidebar-foreground/60">
                <AppSidebar
                  app={app}
                  menus={menusQuery.data ?? []}
                  isLoading={menusQuery.isLoading}
                  isError={menusQuery.isError}
                  onRetry={() => menusQuery.refetch()}
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
          />
          <main
            id="app-scroll"
            className="flex-1 overflow-y-auto scroll-area p-4 md:p-6"
            aria-live="polite"
          >
            <AnimatePresence mode="wait">
              <motion.div
                key={pageTransitionKey}
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.15, ease: "easeOut" }}
                className="mx-auto w-full max-w-[1600px]"
              >
                <Outlet />
              </motion.div>
            </AnimatePresence>
          </main>
        </div>

        <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
      </div>
    </TooltipProvider>
  )
}
