import * as React from "react"
import { useQueryClient } from "@tanstack/react-query"
import { dataGrantApi } from "@/api/data-grants"
import type { DataGrantPreview, DataGrantScope, DataGrantSnapshot, DataGrantTarget } from "@/api/data-grants"
import { assertPreviewMatches, buildGrantChange, changeFingerprint, createGrantDraft } from "@/lib/data-grants"
import { ApiError } from "@/lib/api"
import { isCurrentAuthSession } from "@/lib/auth-queries"
import type { AuthQuerySession } from "@/lib/auth-queries"

export function targetKey(target: DataGrantTarget) {
  return [target.tenantId, target.organizationId, target.userId, target.membershipId, target.applicationId]
}

export function snapshotKey(epoch: number, target: DataGrantTarget) {
  return ["data-grants", "snapshot", epoch, ...targetKey(target)] as const
}

/** Layout cleanup fences a retired editor before another target can paint. */
export function useViewFence(session: AuthQuerySession) {
  const mounted = React.useRef(true)
  React.useLayoutEffect(() => {
    mounted.current = true
    return () => { mounted.current = false }
  }, [])
  return React.useCallback(() => mounted.current && isCurrentAuthSession(session), [session])
}

export async function readInView<T>(current: () => boolean, read: () => Promise<T>): Promise<T> {
  const check = () => { if (!current()) throw new DOMException("授权视图已变化", "AbortError") }
  check()
  try { const value = await read(); check(); return value }
  catch (error) { check(); throw error }
}

type Phase = "ready" | "previewing" | "saving" | "refreshing" | "conflict" | "unknown" | "denied"
type Preview = { data: DataGrantPreview; change: ReturnType<typeof buildGrantChange>; fingerprint: string }
export type SaveReceipt = { snapshot: DataGrantSnapshot; requestId?: string }

export function useGrantEditor(initial: DataGrantSnapshot, session: AuthQuerySession, canWrite: boolean) {
  const queryClient = useQueryClient()
  const currentView = useViewFence(session)
  const generation = React.useRef(0)
  const operation = React.useRef(false)
  const lifetime = React.useRef<AbortController | null>(null)
  React.useLayoutEffect(() => () => lifetime.current?.abort(), [])
  const [snapshot, setSnapshot] = React.useState(initial)
  const [draft, setDraft] = React.useState(() => createGrantDraft(initial))
  const [preview, setPreview] = React.useState<Preview | null>(null)
  const [receipt, setReceipt] = React.useState<SaveReceipt | null>(null)
  const [phase, setPhase] = React.useState<Phase>("ready")
  const [message, setMessage] = React.useState("")
  const [writeDenied, setWriteDenied] = React.useState(false)
  const [readUnavailable, setReadUnavailable] = React.useState(false)
  const busy = phase === "previewing" || phase === "saving" || phase === "refreshing"
  const requiresRead = phase === "conflict" || phase === "unknown"
  const writable = canWrite && !writeDenied && !readUnavailable
  const editable = writable && phase !== "saving" && phase !== "refreshing" && !requiresRead
  const change = buildGrantChange(snapshot, draft)
  const hasChanges = change.upsert.length + change.remove.length > 0
  const fingerprint = changeFingerprint(change)
  const validPreview = preview !== null && preview.fingerprint === fingerprint

  function begin() {
    if (!currentView() || operation.current) return null
    operation.current = true
    lifetime.current?.abort()
    lifetime.current = new AbortController()
    const revision = ++generation.current
    return { signal: lifetime.current.signal, valid: () => currentView() && revision === generation.current }
  }

  function failure(error: unknown, saving: boolean) {
    setPreview(null)
    setDraft(createGrantDraft(snapshot))
    if (error instanceof ApiError && error.status === 409) {
      setPhase("conflict")
      setMessage("授权版本已变化，请重新读取并审阅")
    } else if (error instanceof ApiError && error.status === 403) {
      setWriteDenied(true)
      // A write/preview rejection may also mean the read boundary changed.
      // Retire the old snapshot until a fresh GET independently permits it.
      setReadUnavailable(true)
      setPhase("denied")
      setMessage("写入权限或目标范围已变化，当前仅可查看")
      void queryClient.invalidateQueries({ queryKey: ["permissions", session.epoch] })
    } else if (!(error instanceof ApiError) || error.status === 0 || error.status >= 500) {
      setPhase("unknown")
      setMessage(saving ? "保存结果待确认，请重新读取授权" : "预览未完成，请重新读取授权")
    } else {
      setPhase("ready")
      setMessage(error.message)
    }
  }

  function edit(key: string, scope: DataGrantScope | null) {
    if (!currentView() || (operation.current && phase !== "previewing") || !editable) return
    if (draft[key] === scope) return
    if (phase === "previewing") {
      lifetime.current?.abort()
      operation.current = false
      setPhase("ready")
    }
    generation.current++
    setDraft(previous => ({ ...previous, [key]: scope }))
    setPreview(null)
    setReceipt(null)
    setMessage("")
  }

  async function prepare() {
    if (!editable || busy || !hasChanges) return
    const captured = begin()
    if (!captured) return
    setPreview(null)
    setReceipt(null)
    setMessage("")
    setPhase("previewing")
    try {
      const result = await dataGrantApi.preview(change, captured.signal)
      if (!captured.valid()) return
      assertPreviewMatches(change, result)
      setPreview({ data: result, change, fingerprint })
      setPhase("ready")
    } catch (error) {
      if (captured.valid()) failure(error, false)
    } finally {
      if (captured.valid()) operation.current = false
    }
  }

  async function save() {
    if (!editable || !validPreview || !preview || !hasChanges) return
    const captured = begin()
    if (!captured) return
    setPhase("saving")
    setMessage("")
    try {
      // Confirmation uses the exact delta/version reviewed, never a rebuilt body.
      assertPreviewMatches(preview.change, preview.data)
      const result = await dataGrantApi.change(preview.change, captured.signal)
      if (!captured.valid()) return
      setSnapshot(result.snapshot)
      setDraft(createGrantDraft(result.snapshot))
      setPreview(null)
      setReceipt(result)
      setPhase("ready")
      queryClient.setQueryData(snapshotKey(session.epoch, snapshot.target), result.snapshot)
    } catch (error) {
      if (captured.valid()) failure(error, true)
    } finally {
      if (captured.valid()) operation.current = false
    }
  }

  async function reload() {
    const captured = begin()
    if (!captured) return
    const reconciling = requiresRead
    setPhase("refreshing")
    setPreview(null)
    setReceipt(null)
    setReadUnavailable(true)
    try {
      const result = await queryClient.fetchQuery({
        queryKey: snapshotKey(session.epoch, snapshot.target),
        staleTime: 0,
        retry: false,
        queryFn: async () => {
          const value = await dataGrantApi.read(snapshot.target, captured.signal)
          if (!captured.valid()) throw new DOMException("授权视图已变化", "AbortError")
          return value
        },
      })
      if (!captured.valid()) return
      setSnapshot(result)
      setDraft(createGrantDraft(result))
      setReadUnavailable(false)
      setPhase(writeDenied ? "denied" : "ready")
      setMessage(reconciling ? "已重新读取当前授权；请审阅后重新编辑和预览，本次读取不代表上次保存成功。" : "")
    } catch (error) {
      if (!captured.valid()) return
      setPhase(error instanceof ApiError && error.status === 403 ? "denied" : "unknown")
      if (error instanceof ApiError && error.status === 403) {
        setWriteDenied(true)
        void queryClient.invalidateQueries({ queryKey: ["permissions", session.epoch] })
      }
      setMessage("当前授权读取失败，编辑已暂停，请重新读取授权")
    } finally {
      if (captured.valid()) operation.current = false
    }
  }

  return { snapshot, draft, preview: validPreview ? preview?.data : null,
    previewChange: validPreview ? preview?.change : null, receipt, phase, message,
    writable, editable, busy, requiresRead, readUnavailable, hasChanges, edit, prepare, save, reload }
}
