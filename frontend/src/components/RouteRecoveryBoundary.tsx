import { Component, Suspense, type ReactNode } from "react"
import { Link } from "react-router-dom"

/** A rejected lazy import stays rejected in React's module cache until reload. */
function isChunkFailure(error: Error) {
  return /Failed to fetch dynamically imported module|Importing a module script failed|Loading chunk|ChunkLoadError|error loading dynamically imported module|Unable to preload CSS/i.test(error.message)
}

export function PageLoading() {
  return <div className="flex min-h-[30vh] items-center justify-center text-sm text-muted-foreground" role="status" aria-label="页面加载中">页面加载中…</div>
}

/** Keep navigation outside this boundary; changing its key retires the failed page. */
export class RouteRecoveryBoundary extends Component<{ children: ReactNode; loading?: ReactNode }, { error: Error | null; attempt: number }> {
  state = { error: null as Error | null, attempt: 0 }

  static getDerivedStateFromError(error: Error) { return { error } }

  render() {
    const { error, attempt } = this.state
    if (error) {
      const chunk = isChunkFailure(error)
      return <section role="alert" className="mx-auto my-8 max-w-lg space-y-4 rounded-xl border bg-card p-6">
        <h1 className="text-lg font-semibold">{chunk ? "页面资源加载失败" : "页面暂时无法显示"}</h1>
        <p className="text-sm text-muted-foreground">{chunk ? "资源可能已更新或网络暂时不可用。请重新加载以获取当前版本。" : "可以重试当前页面，或通过导航访问其他页面。"}</p>
        <div className="flex flex-wrap gap-3 text-sm">
          {!chunk && <button type="button" className="rounded-md border px-3 py-2" onClick={() => this.setState({ error: null, attempt: attempt + 1 })}>重试当前页面</button>}
          <button type="button" className="rounded-md border px-3 py-2" onClick={() => window.location.reload()}>重新加载页面</button>
          <Link className="rounded-md border px-3 py-2" to="/dashboard">返回工作台</Link>
        </div>
      </section>
    }
    return <Suspense key={attempt} fallback={this.props.loading ?? <PageLoading />}>{this.props.children}</Suspense>
  }
}
