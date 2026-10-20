import { Fragment } from "react"
import { useAuthStore } from "@/stores/auth"

/** A new session/context physically retires host forms before a new identity renders. */
export function SessionStatusGate({ children }: { children: React.ReactNode }) {
  const status = useAuthStore((state) => state.sessionStatus)
  const error = useAuthStore((state) => state.coordinationError)
  const sessionId = useAuthStore((state) => state.sessionId)
  const epoch = useAuthStore((state) => state.contextEpoch)
  const recovering = status === "unknown" && window.location.pathname === "/login"

  if (status === "initializing" || status === "switching" || status === "unavailable" || (status === "unknown" && !recovering)) {
    return (
      <main className="flex min-h-dvh items-center justify-center bg-background p-6" role="status" aria-live="polite">
        <div className="w-full max-w-md space-y-4 rounded-xl border bg-card p-6 text-card-foreground">
          <h1 className="text-lg font-semibold">
            {status === "initializing" ? "正在恢复共享会话" : status === "switching" ? "正在同步登录与组织" : status === "unavailable" ? "当前浏览器无法安全共享登录" : "会话结果尚未确认"}
          </h1>
          <p className="text-sm leading-relaxed text-muted-foreground">
            {status === "initializing" || status === "switching" ? "正在同步其他标签页的登录与组织，请稍候。" : error || (status === "unavailable"
              ? "请使用支持安全会话协调的 Chrome 或 Edge，并允许此站点保存本地数据。"
              : "为避免使用旧组织数据，当前页面已暂停。请重新登录以确认会话。")}
          </p>
          {status === "unavailable" && <p className="text-sm text-muted-foreground">请在 HTTPS 或本机地址访问，并启用 Web Locks 和本地存储后重新加载。</p>}
          {status === "unknown" && <a href="/login" className="inline-flex rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground">重新登录</a>}
          {status === "unavailable" && <button type="button" onClick={() => window.location.reload()} className="inline-flex rounded-md border px-4 py-2 text-sm">重新检查</button>}
        </div>
      </main>
    )
  }

  // Version-only token renewal keeps this key, preserving drafts in the same
  // app/tenant/organization. Context changes discard the old ephemeral drafts.
  return <Fragment key={`${sessionId ?? "anonymous"}:${epoch}`}>{children}</Fragment>
}
