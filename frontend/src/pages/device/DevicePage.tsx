import * as React from "react"
import { Navigate, useSearchParams } from "react-router-dom"
import { useMutation } from "@tanstack/react-query"
import { motion } from "framer-motion"
import { AlertCircle, CheckCircle2, Loader2, ShieldCheck, Tv, XCircle } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { BrandWordmark } from "@/components/brand"
import { deviceApi } from "@/api/public"
import { useAuthStore } from "@/stores/auth"
import { useDocumentTitle } from "@/hooks/use-document-title"
import { SCOPE_LABELS, formatRelative } from "@/lib/format"
import { AppIcon } from "@/lib/icons"
import { toast } from "sonner"
import type { DeviceInfo } from "@/api/types"

/** 归一化设备码:大写 + 自动插 '-'(XXXX-XXXX) */
function normalizeCode(raw: string): string {
  const clean = raw.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 8)
  return clean.length > 4 ? `${clean.slice(0, 4)}-${clean.slice(4)}` : clean
}

export default function DevicePage() {
  const token = useAuthStore((s) => s.token)
  const [params] = useSearchParams()
  const returnTo = `/device${params.get("user_code") ? `?user_code=${params.get("user_code")}` : ""}`
  useDocumentTitle("设备授权")

  const [code, setCode] = React.useState(() => normalizeCode(params.get("user_code") ?? ""))
  const [info, setInfo] = React.useState<DeviceInfo | null>(null)
  const [error, setError] = React.useState<string | null>(null)
  const [result, setResult] = React.useState<"approved" | "denied" | "failed" | null>(null)
  const [resultMsg, setResultMsg] = React.useState<string | null>(null)

  const lookup = useMutation({
    mutationFn: (userCode: string) => deviceApi.info(userCode),
    onSuccess: (data) => {
      setError(null)
      setInfo(data)
    },
    onError: (e: Error) => {
      setInfo(null)
      setError(e.message || "设备码查询失败")
    },
  })

  const decide = useMutation({
    mutationFn: ({ userCode, approve }: { userCode: string; approve: boolean }) =>
      deviceApi.decide(userCode, approve),
    onSuccess: (_d, vars) => {
      setResult(vars.approve ? "approved" : "denied")
    },
    onError: (e: Error) => {
      setResult("failed")
      setResultMsg(e.message || "决策提交失败")
    },
  })

  if (!token) {
    return <Navigate to={`/login?return=${encodeURIComponent(returnTo)}`} replace />
  }

  const submit = (e: React.FormEvent) => {
    e.preventDefault()
    const normalized = normalizeCode(code)
    if (normalized.replace("-", "").length !== 8) {
      setError("请输入完整的 8 位设备码(格式:XXXX-XXXX)")
      return
    }
    setCode(normalized)
    lookup.mutate(normalized)
  }

  const scopes = (info?.scope ?? "").split(/\s+/).filter(Boolean)

  return (
    <div className="flex min-h-dvh flex-col bg-muted/40">
      <header className="border-b bg-background px-4 py-3">
        <div className="mx-auto flex max-w-3xl items-center justify-between">
          <BrandWordmark />
          <span className="text-xs text-muted-foreground">设备授权 · RFC 8628</span>
        </div>
      </header>

      <main className="flex flex-1 items-center justify-center px-4 py-10">
        <div className="w-full max-w-md">
          {result === null && !info && (
            <motion.div
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              className="rounded-2xl border bg-card p-7 shadow-sm"
            >
              <div className="mb-6 text-center">
                <span className="mx-auto mb-3 flex size-12 items-center justify-center rounded-xl bg-primary/10 text-primary">
                  <Tv className="size-6" />
                </span>
                <h1 className="text-xl font-semibold tracking-tight">设备授权</h1>
                <p className="mt-1.5 text-sm text-muted-foreground">
                  请输入电视或其他设备屏幕上显示的 8 位设备码
                </p>
              </div>

              {error && (
                <Alert variant="destructive" className="mb-4">
                  <AlertCircle className="size-4" />
                  <AlertDescription>{error}</AlertDescription>
                </Alert>
              )}

              <form onSubmit={submit} className="space-y-4">
                <div className="space-y-2">
                  <Label htmlFor="user_code">
                    设备码 <span className="text-destructive">*</span>
                  </Label>
                  <Input
                    id="user_code"
                    value={code}
                    onChange={(e) => setCode(normalizeCode(e.target.value))}
                    placeholder="XXXX-XXXX"
                    inputMode="text"
                    autoComplete="off"
                    autoFocus
                    className="h-12 text-center font-mono text-xl tracking-[0.35em] uppercase"
                  />
                  <p className="text-xs text-muted-foreground">输入时自动大写并格式化</p>
                </div>
                <Button type="submit" className="h-10 w-full" disabled={lookup.isPending}>
                  {lookup.isPending && <Loader2 className="size-4 animate-spin" />}
                  查询设备
                </Button>
              </form>
            </motion.div>
          )}

          {result === null && info && (
            <motion.div
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              className="rounded-2xl border bg-card p-7 shadow-sm"
            >
              <div className="mb-5 flex items-center gap-3.5">
                <span className="flex size-12 items-center justify-center rounded-xl bg-primary/10 text-primary">
                  <Tv className="size-6" />
                </span>
                <div className="min-w-0">
                  <p className="text-xs text-muted-foreground">设备码 {code} 对应的客户端</p>
                  <h2 className="truncate text-lg font-semibold">{info.clientName}</h2>
                  <p className="truncate font-mono text-xs text-muted-foreground">{info.clientId}</p>
                </div>
              </div>

              <div className="mb-5 rounded-xl border bg-muted/40 p-4">
                <p className="mb-2.5 text-xs font-medium text-muted-foreground">该客户端申请以下权限:</p>
                <ul className="space-y-2">
                  {scopes.length === 0 && <li className="text-sm text-muted-foreground">未申请任何权限</li>}
                  {scopes.map((s) => {
                    const meta = SCOPE_LABELS[s]
                    return (
                      <li key={s} className="flex items-center gap-2.5 text-sm">
                        <AppIcon name={meta?.icon ?? "CircleDot"} className="size-4 text-primary" />
                        <span>{meta?.label ?? s}</span>
                        <span className="truncate text-xs text-muted-foreground">{meta?.desc ?? "自定义权限范围"}</span>
                      </li>
                    )
                  })}
                </ul>
              </div>

              {info.status === "PENDING" ? (
                <>
                  <p className="mb-4 text-xs text-muted-foreground">
                    授权请求有效期至 {formatRelative(info.expiresAt)},批准后设备将自动获得访问令牌。
                  </p>
                  {error && (
                    <Alert variant="destructive" className="mb-4">
                      <AlertCircle className="size-4" />
                      <AlertDescription>{error}</AlertDescription>
                    </Alert>
                  )}
                  <div className="flex gap-3">
                    <Button
                      variant="outline"
                      className="h-10 flex-1"
                      disabled={decide.isPending}
                      onClick={() => decide.mutate({ userCode: code, approve: false })}
                    >
                      {decide.isPending && decide.variables?.approve === false && (
                        <Loader2 className="size-4 animate-spin" />
                      )}
                      拒绝
                    </Button>
                    <Button
                      className="h-10 flex-1"
                      disabled={decide.isPending}
                      onClick={() => decide.mutate({ userCode: code, approve: true })}
                    >
                      {decide.isPending && decide.variables?.approve === true && (
                        <Loader2 className="size-4 animate-spin" />
                      )}
                      <ShieldCheck className="size-4" />
                      批准授权
                    </Button>
                  </div>
                </>
              ) : (
                <Alert>
                  <AlertCircle className="size-4" />
                  <AlertTitle>该设备码不可授权</AlertTitle>
                  <AlertDescription>
                    当前状态:{info.status === "APPROVED" ? "已批准" : info.status === "DENIED" ? "已拒绝" : "已过期"}
                    ,如需重新授权请在设备上发起新的授权请求。
                  </AlertDescription>
                </Alert>
              )}
            </motion.div>
          )}

          {result !== null && (
            <motion.div
              initial={{ opacity: 0, scale: 0.96 }}
              animate={{ opacity: 1, scale: 1 }}
              transition={{ type: "spring", stiffness: 260, damping: 20 }}
              className="rounded-2xl border bg-card p-10 text-center shadow-sm"
            >
              <motion.div
                initial={{ scale: 0 }}
                animate={{ scale: 1 }}
                transition={{ delay: 0.1, type: "spring", stiffness: 300, damping: 16 }}
                className={`mx-auto mb-4 flex size-16 items-center justify-center rounded-full ${
                  result === "approved"
                    ? "bg-primary/10 text-primary"
                    : result === "denied"
                      ? "bg-muted text-muted-foreground"
                      : "bg-destructive/10 text-destructive"
                }`}
              >
                {result === "approved" ? (
                  <CheckCircle2 className="size-9" />
                ) : (
                  <XCircle className="size-9" />
                )}
              </motion.div>
              <h2 className="text-xl font-semibold tracking-tight">
                {result === "approved" ? "授权成功" : result === "denied" ? "已拒绝授权" : "授权失败"}
              </h2>
              <p className="mx-auto mt-2 max-w-xs text-sm text-muted-foreground">
                {result === "approved"
                  ? "设备已获得访问权限,请回到设备端继续操作。"
                  : result === "denied"
                    ? "您已拒绝该设备的授权请求,设备端将收到拒绝结果。"
                    : resultMsg || "处理请求时出现错误,请稍后重试。"}
              </p>
              <Button
                variant="outline"
                className="mt-6"
                onClick={() => {
                  setResult(null)
                  setInfo(null)
                  setError(null)
                  setCode("")
                }}
              >
                授权另一台设备
              </Button>
            </motion.div>
          )}
        </div>
      </main>

      <footer className="border-t bg-background px-4 py-3 text-center text-xs text-muted-foreground">
        © 2026 Dante Cloud · Powered by Rust &amp; React
      </footer>
    </div>
  )
}
