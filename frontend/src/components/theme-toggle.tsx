import * as React from "react"
import { Laptop, Moon, Sun } from "lucide-react"
import { useTheme } from "@/components/theme-provider"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

const OPTIONS = [
  { value: "light", label: "浅色", icon: Sun },
  { value: "dark", label: "深色", icon: Moon },
  { value: "system", label: "跟随系统", icon: Laptop },
] as const

export function ThemeToggle({ className }: { className?: string }) {
  const { theme, setTheme } = useTheme()
  const next = () => {
    const order = ["light", "dark", "system"] as const
    const idx = order.indexOf(theme as (typeof order)[number])
    setTheme(order[(idx + 1) % order.length])
  }
  const Current = theme === "light" ? Sun : theme === "dark" ? Moon : Laptop
  return (
    <Button
      variant="ghost"
      size="icon"
      aria-label={`切换主题(当前:${theme === "light" ? "浅色" : theme === "dark" ? "深色" : "跟随系统"})`}
      title={`切换主题(当前:${theme === "light" ? "浅色" : theme === "dark" ? "深色" : "跟随系统"})`}
      onClick={next}
      className={className}
    >
      <Current className="size-4.5" />
    </Button>
  )
}

/** 带下拉的三态主题切换(用于登录页等空间充裕处) */
export function ThemeToggleGroup({ className }: { className?: string }) {
  const { theme, setTheme } = useTheme()
  return (
    <div className={cn("inline-flex items-center gap-1 rounded-lg border bg-card p-1", className)} role="group" aria-label="主题切换">
      {OPTIONS.map((o) => (
        <button
          key={o.value}
          type="button"
          aria-label={o.label}
          title={o.label}
          onClick={() => setTheme(o.value)}
          className={cn(
            "inline-flex size-7 items-center justify-center rounded-md transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            theme === o.value
              ? "bg-primary/10 text-primary"
              : "text-muted-foreground hover:text-foreground hover:bg-muted"
          )}
        >
          <o.icon className="size-4" />
        </button>
      ))}
    </div>
  )
}
