import * as React from "react"
import { Navigate, useSearchParams } from "react-router-dom"
import { useMutation, useQuery } from "@tanstack/react-query"
import { motion } from "framer-motion"
import { AlertCircle, CheckCircle2, ExternalLink, KeyRound, Loader2, ShieldCheck, XCircle } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { BrandWordmark } from "@/components/brand"
import { consentApi } from "@/api/public"
import { useAuthStore } from "@/stores/auth"
import { useDocumentTitle } from "@/hooks/use-document-title"
import { SCOPE_LABELS } from "@/lib/format"
import { AppIcon } from "@/lib/icons"
import type { ConsentInfo } from "@/api/types"

export default function ConsentPage() {
  const token = useAuthStore((s) => s.token)
  const [params] = useSearchParams()
  const requestId = params.get("request_id") ?? ""
  const returnTo = `/consent${requestId ? `?request_id=${requestId}` : ""}`
  useDocumentTitle("授权确认")

  const infoQuery = useQuery({
    queryKey: ["consent", requestId],
    queryFn: () => consentApi.info(requestId),
    enabled: !!requestId && !!token,
    retry: 1,
    staleTime: 30_000,
  })

  const decide = useMutation({
    mutationFn: (approve: boolean) => consentApi.decide(requestId, approve),
    onSuccess: (data) => {
      // 跳回客户端回调地址
      window.location.href = data.redirectTo
    },
  })

  if (!token) {
    return <Navigate to={`/login?return=${encodeURIComponent(returnTo)}`} replace />
  }

  if (!requestId) {
    return (
      <ConsentShell>
        <MissingRequest
          message="缺少 request_id 参数。请从客户端的授权链接重新发起授权。"
        />
      </ConsentShell>
    )
  }

  if (infoQuery.isLoading) {
    return (
      <ConsentShell>
        <div className="flex flex-col items-center gap-3 py-14 text-muted-foreground">
          <Loader2 className="size-6 animate-spin text-primary" />
          <p className="text-sm">正在加载授权请求...</p>
        </div>
      </ConsentShell>
    )
  }

  if (infoQuery.isError) {
    return (
      <ConsentShell>
        <MissingRequest
          message={
            infoQuery.error instanceof Error
              ? `${infoQuery.error.message}。该授权请求可能已过期或已被使用,请回到客户端重新发起授权。`
              : "授权请求无效或已过期,请回到客户端重新发起授权。"
          }
          onRetry={() => infoQuery.refetch()}
        />
      </ConsentShell>
    )
  }

  const info: ConsentInfo = infoQuery.data!
  const scopes = info.scope.split(/\s+/).filter(Boolean)
  const redirectHost = (() => {
    try {
      return new URL(info.redirectUri).host
    } catch {
      return info.redirectUri
    }
  })()

  return (
    <ConsentShell>
      <motion.div initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3 }}>
        <div className="mb-6 text-center">
          <h1 className="text-xl font-semibold tracking-tight">授权确认</h1>
          <p className="mt-1.5 text-sm text-muted-foreground">确认将以下权限授予第三方应用</p>
        </div>

        {/* 客户端卡片 */}
        <div className="mb-5 flex items-center gap-4 rounded-xl border bg-muted/40 p-4">
          <span className="flex size-12 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <KeyRound className="size-6" />
          </span>
          <div className="min-w-0 flex-1">
            <p className="text-sm text-muted-foreground">将授权给</p>
            <p className="truncate text-base font-semibold">{info.client.clientName}</p>
            {info.client.description && (
              <p className="truncate text-xs text-muted-foreground">{info.client.description}</p>
            )}
          </div>
        </div>

        {/* 权限清单 */}
        <div className="mb-4 rounded-xl border p-4">
          <p className="mb-3 text-xs font-medium text-muted-foreground">该应用将获得以下权限:</p>
          <ul className="space-y-2.5">
            {scopes.length === 0 && <li className="text-sm text-muted-foreground">未申请任何权限</li>}
            {scopes.map((s) => {
              const meta = SCOPE_LABELS[s]
              return (
                <li key={s} className="flex items-start gap-3">
                  <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                    <AppIcon name={meta?.icon ?? "CircleDot"} className="size-4" />
                  </span>
                  <div className="min-w-0">
                    <p className="text-sm font-medium">{meta?.label ?? s}</p>
                    <p className="text-xs text-muted-foreground">{meta?.desc ?? "自定义权限范围"}</p>
                  </div>
                </li>
              )
            })}
          </ul>
        </div>

        <p className="mb-5 flex items-start gap-1.5 text-xs text-muted-foreground">
          <ExternalLink className="mt-0.5 size-3.5 shrink-0" />
          <span>
            授权后将跳转至目标应用
            <span className="mx-1 font-medium text-foreground">{redirectHost}</span>
            {info.state ? `(state: ${info.state.slice(0, 12)}...)` : ""}
          </span>
        </p>

        {decide.isError && (
          <Alert variant="destructive" className="mb-4">
            <AlertCircle className="size-4" />
            <AlertDescription>
              {decide.error instanceof Error ? decide.error.message : "决策提交失败,请重试"}
            </AlertDescription>
          </Alert>
        )}

        <div className="flex gap-3">
          <Button
            variant="outline"
            className="h-10 flex-1"
            disabled={decide.isPending}
            onClick={() => decide.mutate(false)}
          >
            {decide.isPending && decide.variables === false && <Loader2 className="size-4 animate-spin" />}
            拒绝
          </Button>
          <Button
            className="h-10 flex-1"
            disabled={decide.isPending}
            onClick={() => decide.mutate(true)}
          >
            {decide.isPending && decide.variables === true ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <ShieldCheck className="size-4" />
            )}
            同意授权
          </Button>
        </div>
      </motion.div>
    </ConsentShell>
  )
}

function ConsentShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex min-h-dvh flex-col bg-muted/40">
      <header className="border-b bg-background px-4 py-3">
        <div className="mx-auto flex max-w-3xl items-center justify-between">
          <BrandWordmark />
          <span className="text-xs text-muted-foreground">OAuth 2.0 授权确认</span>
        </div>
      </header>
      <main className="flex flex-1 items-center justify-center px-4 py-10">
        <div className="w-full max-w-md rounded-2xl border bg-card p-7 shadow-sm">{children}</div>
      </main>
      <footer className="border-t bg-background px-4 py-3 text-center text-xs text-muted-foreground">
        © 2026 Dante Cloud · Powered by Rust &amp; React
      </footer>
    </div>
  )
}

function MissingRequest({ message, onRetry }: { message: string; onRetry?: () => void }) {
  return (
    <div className="text-center">
      <div className="mx-auto mb-4 flex size-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <XCircle className="size-7" />
      </div>
      <h1 className="text-lg font-semibold tracking-tight">授权请求不可用</h1>
      <Alert variant="destructive" className="mt-4 text-left">
        <AlertCircle className="size-4" />
        <AlertTitle>无法完成授权</AlertTitle>
        <AlertDescription>{message}</AlertDescription>
      </Alert>
      {onRetry && (
        <Button variant="outline" className="mt-5" onClick={onRetry}>
          重新加载
        </Button>
      )}
      <div className="mt-6 flex items-center justify-center gap-1.5 text-xs text-muted-foreground">
        <CheckCircle2 className="size-3.5" />
        授权请求有效期 10 分钟,且仅可使用一次
      </div>
    </div>
  )
}
