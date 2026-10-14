import * as React from "react"
import { PanelLeft, PanelLeftClose } from "lucide-react"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { AppIcon } from "@/lib/icons"
import { useTheme } from "@/components/theme-provider"
import { cn } from "@/lib/utils"
import type { AppBrief } from "@/api/types"

/** 左侧应用导航轨(68px,深色) */
export function AppRail({
  apps,
  currentApplicationId,
  onSelect,
  sidebarOpen,
  onToggleSidebar,
  className,
}: {
  apps: AppBrief[]
  currentApplicationId: string | null
  onSelect: (appId: string) => void
  sidebarOpen: boolean
  onToggleSidebar: () => void
  className?: string
}) {
  const { resolvedTheme, setTheme } = useTheme()

  return (
    <TooltipProvider delayDuration={200}>
      <aside
        aria-label="应用导航"
        className={cn(
          "flex w-[68px] shrink-0 flex-col items-center gap-1 border-r bg-sidebar py-3 text-sidebar-foreground",
          className
        )}
      >
        <div className="flex flex-1 flex-col items-center gap-1 overflow-y-auto scroll-area w-full px-2">
          {apps.map((app) => {
            const active = app.id === currentApplicationId
            return (
              <Tooltip key={app.id}>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    aria-label={`切换到应用 ${app.name}`}
                    aria-current={active ? "true" : undefined}
                    onClick={() => onSelect(app.id)}
                    className={cn(
                      "group relative flex size-11 shrink-0 items-center justify-center rounded-xl outline-none transition-colors focus-visible:ring-2 focus-visible:ring-sidebar-ring",
                      active
                        ? "bg-sidebar-primary/20 text-sidebar-primary"
                        : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
                    )}
                  >
                    <AppIcon name={app.icon} size={22} />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="right" sideOffset={8} className="text-xs">
                  <span className="flex max-w-[10rem] items-center gap-1.5">
                    <span className="max-w-[7rem] truncate font-medium" title={app.name}>
                      {app.name}
                    </span>
                    <span className="font-mono opacity-60">{app.code}</span>
                  </span>
                </TooltipContent>
              </Tooltip>
            )
          })}
        </div>

        <div className="mt-2 flex w-full flex-col items-center gap-1 border-t border-sidebar-border pt-2">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                aria-label={sidebarOpen ? "折叠侧栏" : "展开侧栏"}
                onClick={onToggleSidebar}
                className="flex size-11 items-center justify-center rounded-xl text-sidebar-foreground/70 outline-none transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring"
              >
                {sidebarOpen ? <PanelLeftClose className="size-5" /> : <PanelLeft className="size-5" />}
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8} className="text-xs">
              {sidebarOpen ? "折叠侧栏" : "展开侧栏"}
            </TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                aria-label={resolvedTheme === "dark" ? "切换到浅色模式" : "切换到深色模式"}
                onClick={() => setTheme(resolvedTheme === "dark" ? "light" : "dark")}
                className="flex size-11 items-center justify-center rounded-xl text-sidebar-foreground/70 outline-none transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring"
              >
                {resolvedTheme === "dark" ? (
                  <svg viewBox="0 0 24 24" className="size-5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
                    <circle cx="12" cy="12" r="4" />
                    <path d="M12 2v2m0 16v2M4.93 4.93l1.41 1.41m11.32 11.32 1.41 1.41M2 12h2m16 0h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
                  </svg>
                ) : (
                  <svg viewBox="0 0 24 24" className="size-5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
                    <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
                  </svg>
                )}
              </button>
            </TooltipTrigger>
            <TooltipContent side="right" sideOffset={8} className="text-xs">
              {resolvedTheme === "dark" ? "浅色模式" : "深色模式"}
            </TooltipContent>
          </Tooltip>
        </div>
      </aside>
    </TooltipProvider>
  )
}
