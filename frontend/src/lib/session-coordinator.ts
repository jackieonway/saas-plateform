import { newUuid } from "@/lib/uuid"
import { contextIdentityKey, isContextIdentity, sameContextIdentity } from "@/lib/context-identity"
import type { ContextIdentity } from "@/lib/context-identity"
/** One durable credential record and one browser lock for every platform session mutation.
 * Notifications carry versions only; storage is always the source of truth. */
export const SESSION_KEY = "platform-session-v2"
// Retain the global lock name so older pages cannot consume credentials in
// parallel with the identity-bound record migration.
export const SESSION_LOCK = "platform-session-coordinator-v1"
export const SESSION_CHANNEL = "platform-session-version-v2"
const PREVIOUS_SESSION_KEY = "platform-session-v1"
const LEGACY_TOKEN_KEY = "platform-token"
const LEGACY_REFRESH_KEY = "platform-refresh-token"

export interface SessionFence { readonly sessionId: string | null; readonly version: number; readonly contextKey: string | null }
export interface SessionTokens { token: string; refreshToken: string | null; identity: ContextIdentity }
export type SessionOperation = "refresh" | "context" | "logout"
export interface SessionRecord {
  schema: 2
  version: number
  sessionId: string | null
  status: "active" | "anonymous" | "in-flight" | "unknown"
  token: string | null
  refreshToken: string | null
  identity: ContextIdentity | null
  contextKey: string | null
  intent?: { opId: string; baseVersion: number; kind: SessionOperation }
}
export type SessionStatus = "initializing" | "active" | "anonymous" | "refreshing" | "switching" | "unknown" | "unavailable"
export interface SessionObservation { record: SessionRecord | null; status: SessionStatus; error: string | null }

interface SessionEnvironment {
  storage: Pick<Storage, "getItem" | "setItem" | "removeItem">
  locks: { request<T>(name: string, callback: () => Promise<T>): Promise<T> }
  id: () => string
  notify?: (version: number) => void
  listen?: (changed: () => void) => () => void
}

export class SessionCoordinationError extends Error {
  constructor(message: string, readonly code: "SESSION_UNAVAILABLE" | "SESSION_UNKNOWN" | "SESSION_CHANGED") {
    super(message)
    this.name = code === "SESSION_CHANGED" ? "AbortError" : "SessionCoordinationError"
  }
}

/** Only a definite protocol rejection may release an intent for reuse. An HTTP
 * 5xx, unreadable success, transport loss or owner departure cannot prove rollback. */
export class SessionOperationError extends Error {
  constructor(message: string, readonly disposition: "invalid" | "safe" | "unknown", readonly status = 0) { super(message) }
}

function parseRecord(raw: string): SessionRecord {
  const r = JSON.parse(raw) as SessionRecord
  const statuses = ["active", "anonymous", "in-flight", "unknown"]
  if (r?.schema !== 2 || !Number.isSafeInteger(r.version) || r.version < 0 || !statuses.includes(r.status) ||
      !(r.sessionId === null || typeof r.sessionId === "string") ||
      !(r.token === null || typeof r.token === "string") || !(r.refreshToken === null || typeof r.refreshToken === "string") ||
      (r.status !== "anonymous" && (!r.sessionId || !r.token || !isContextIdentity(r.identity) ||
        r.identity.memberType !== "EMPLOYEE" || r.contextKey !== contextIdentityKey(r.identity))) ||
      (r.status === "anonymous" && (r.sessionId !== null || r.token !== null || r.refreshToken !== null || r.identity !== null || r.contextKey !== null)) ||
      (r.status === "in-flight" && (!r.intent || typeof r.intent.opId !== "string" ||
        !Number.isSafeInteger(r.intent.baseVersion) || !["refresh", "context", "logout"].includes(r.intent.kind)))) {
    throw new Error("Invalid platform session record")
  }
  return r
}

function anonymous(version: number): SessionRecord {
  return { schema: 2, version, sessionId: null, status: "anonymous", token: null, refreshToken: null, identity: null, contextKey: null }
}

function checkedTokens(tokens: SessionTokens): SessionTokens {
  if (typeof tokens.token !== "string" || !tokens.token || !(tokens.refreshToken === null || typeof tokens.refreshToken === "string") ||
      !isContextIdentity(tokens.identity) || tokens.identity.memberType !== "EMPLOYEE") throw new Error("会话凭据或员工成员证明缺失")
  const { userId, tenantId, organizationId, membershipId, memberType, identityVersion, accountVersion } = tokens.identity
  return { token: tokens.token, refreshToken: tokens.refreshToken,
    identity: Object.freeze({ userId, tenantId, organizationId, membershipId, memberType, identityVersion, accountVersion }) }
}

export function createSessionCoordinator(environment: SessionEnvironment | null) {
  let observation: SessionObservation = { record: null, status: "initializing", error: null }
  const observers = new Set<(value: SessionObservation) => void>()
  let listening = false
  let active: { controller: AbortController; opId: string } | null = null
  const unavailableMessage = "此浏览器无法安全协调共享会话，请启用持久存储并使用支持 Web Locks 的 Chrome 或 Edge。"
  const unknownMessage = "上次会话操作的结果无法确认。为避免重复使用凭据，已停止平台请求，请重新登录。"

  function publish(record: SessionRecord | null, unavailable = false) {
    const status: SessionStatus = unavailable ? "unavailable" : !record ? "initializing" :
      record.status === "in-flight" ? record.intent?.kind === "refresh" ? "refreshing" : "switching" : record.status
    const error = unavailable ? unavailableMessage : status === "unknown" ? unknownMessage : null
    if (JSON.stringify(observation.record) === JSON.stringify(record) && observation.status === status && observation.error === error) return
    observation = { record, status, error }
    observers.forEach(observer => observer(observation))
  }
  function unavailable(): never {
    publish(observation.record, true)
    throw new SessionCoordinationError(unavailableMessage, "SESSION_UNAVAILABLE")
  }
  function env(): SessionEnvironment { if (!environment?.locks || !environment.storage || !environment.id) return unavailable(); return environment }
  function read(): SessionRecord | null {
    try { const raw = env().storage.getItem(SESSION_KEY); return raw === null ? null : parseRecord(raw) } catch { return unavailable() }
  }
  function observe(): void {
    // A readable record does not prove that a previous failed durable write is
    // writable again. Recovery requires reloading with restored capabilities.
    if (observation.status === "unavailable") return
    try { publish(read()) } catch { /* The observation already contains the explicit capability error. */ }
  }
  function write(record: SessionRecord): SessionRecord {
    try {
      const raw = JSON.stringify(record)
      parseRecord(raw)
      env().storage.setItem(SESSION_KEY, raw)
      if (env().storage.getItem(SESSION_KEY) !== raw) return unavailable()
    } catch { return unavailable() }
    publish(record)
    // A failed optional notification never changes a successful durable commit.
    try { environment?.notify?.(record.version) } catch { /* storage events and reconciliation remain available */ }
    return record
  }
  function loadLocked(): SessionRecord {
    let record = read()
    if (!record) {
      try {
        // Neither v1 records nor split legacy tokens prove a concrete member.
        // They must require a fresh authenticated server context, never refresh.
        record = write(anonymous(0))
      } catch { return unavailable() }
    }
    // Migration is one-way. Even after logout an anonymous record remains, so
    // stale legacy keys can never resurrect or become a second token channel.
    try { env().storage.removeItem(PREVIOUS_SESSION_KEY); env().storage.removeItem(LEGACY_TOKEN_KEY); env().storage.removeItem(LEGACY_REFRESH_KEY) } catch { return unavailable() }
    if (record.status === "in-flight") record = write({ ...record, version: record.version + 1, status: "unknown" })
    publish(record)
    return record
  }
  async function locked<T>(action: (record: SessionRecord) => Promise<T> | T): Promise<T> {
    observe()
    if (observation.status === "unavailable") return unavailable()
    let entered = false
    try { return await env().locks.request(SESSION_LOCK, async () => { entered = true; return action(loadLocked()) }) }
    catch (error) { if (!entered) return unavailable(); throw error }
  }
  function changed(): never { throw new SessionCoordinationError("登录上下文已变化", "SESSION_CHANGED") }
  function matches(record: SessionRecord, expected: SessionFence | undefined, exact = false): boolean {
    return !expected || (record.sessionId === expected.sessionId && record.contextKey === expected.contextKey && (!exact || record.version === expected.version))
  }
  function usable(record: SessionRecord): asserts record is SessionRecord & { token: string; sessionId: string } {
    if (record.status === "unknown") throw new SessionCoordinationError(unknownMessage, "SESSION_UNKNOWN")
    if (record.status !== "active" || !record.token || !record.sessionId || !record.identity || !record.contextKey) changed()
  }
  function stopLocalOperation(expected?: SessionFence) {
    if (!active) return
    const record = read()
    // A stale UI callback has no authority to interrupt the replacement session.
    if (record && record.intent?.opId === active.opId && matches(record, expected, expected?.sessionId === null)) active.controller.abort()
  }

  async function initialize() {
    if (!listening) {
      env()
      listening = true
      environment?.listen?.(() => {
        observe()
        // An abandoned intent is recovered only after obtaining the actual
        // browser lock; a slow owner must never be mistaken for a dead owner.
        if (observation.record?.status === "in-flight") void reconcile().catch(() => {})
      })
    }
    await locked(() => {})
  }
  async function reconcile(): Promise<SessionRecord> { return locked(record => record) }
  async function replace(tokens: SessionTokens, expected?: SessionFence): Promise<boolean> {
    const pair = checkedTokens(tokens)
    stopLocalOperation(expected)
    return locked(record => {
      if (!matches(record, expected, expected?.sessionId === null)) return false
      write({ schema: 2, version: record.version + 1, sessionId: env().id(), status: "active", ...pair,
        contextKey: contextIdentityKey(pair.identity) })
      return true
    })
  }
  async function logout(expected?: SessionFence): Promise<boolean> {
    stopLocalOperation(expected)
    return locked(record => {
      if (!matches(record, expected)) return false
      write(anonymous(record.version + 1))
      return true
    })
  }
  async function operation<T>(kind: SessionOperation, expected: SessionFence,
    perform: (scope: { record: SessionRecord & { token: string; sessionId: string }; signal: AbortSignal;
      updateTokens: (tokens: SessionTokens) => void }) => Promise<{ value: T; tokens?: SessionTokens }>): Promise<T | SessionRecord> {
    return locked(async latest => {
      if (!matches(latest, expected)) return changed()
      usable(latest)
      // A waiter always reuses the winner's complete pair instead of consuming
      // the same refresh token. This comparison takes place inside Web Locks.
      if (kind === "refresh" && latest.version !== expected.version) return latest
      const opId = env().id(), controller = new AbortController()
      active = { opId, controller }
      let timedOut = false
      let working = write({ ...latest, version: latest.version + 1, status: "in-flight",
        intent: { opId, kind, baseVersion: latest.version } })
      const deadline = setTimeout(() => { timedOut = true; controller.abort() }, 20_000)
      const assertOwner = () => {
        const current = read()
        if (controller.signal.aborted || active?.opId !== opId || current?.intent?.opId !== opId || current.status !== "in-flight" ||
            !matches(current, { sessionId: working.sessionId, version: working.version, contextKey: working.contextKey }, true)) changed()
      }
      const aborted = new Promise<never>((_, reject) => controller.signal.addEventListener("abort", () =>
        reject(new SessionCoordinationError(timedOut ? unknownMessage : "登录上下文已变化", timedOut ? "SESSION_UNKNOWN" : "SESSION_CHANGED")), { once: true }))
      try {
        const result = await Promise.race([perform({ record: { ...latest }, signal: controller.signal, updateTokens: tokens => {
          assertOwner()
          let pair: SessionTokens
          try { pair = checkedTokens(tokens) } catch {
            throw new SessionOperationError("会话凭据响应无效", "unknown")
          }
          if (!sameContextIdentity(pair.identity, latest.identity)) throw new SessionOperationError("刷新不能更换成员身份", "unknown")
          working = write({ ...working, ...pair, version: working.version + 1 })
        } }), aborted])
        assertOwner()
        if (kind === "logout") {
          write(anonymous(working.version + 1))
        } else {
          let pair: SessionTokens | undefined
          try { pair = result.tokens ? checkedTokens(result.tokens) : undefined } catch {
            throw new SessionOperationError("会话凭据响应无效", "unknown")
          }
          if (kind === "context" && !pair) throw new SessionOperationError("成员切换响应缺少证明", "unknown")
          if (kind === "refresh" && pair && !sameContextIdentity(pair.identity, latest.identity)) throw new SessionOperationError("刷新不能更换成员身份", "unknown")
          const base = { ...working }; delete base.intent
          write({ ...base, ...pair, contextKey: pair ? contextIdentityKey(pair.identity) : working.contextKey, version: working.version + 1, status: "active",
            sessionId: kind === "context" ? env().id() : working.sessionId })
        }
        return result.value
      } catch (error) {
        // Only our still-current durable intent may be settled. Old responses
        // cannot clear a replacement session, including after explicit abort.
        const current = read()
        if (current?.status === "in-flight" && current.intent?.opId === opId &&
            matches(current, { sessionId: working.sessionId, version: working.version, contextKey: working.contextKey }, true)) {
          if (error instanceof SessionOperationError && error.disposition === "invalid") {
            write(anonymous(current.version + 1))
          } else if (error instanceof SessionOperationError && error.disposition === "safe") {
            const base = { ...current }; delete base.intent
            write({ ...base, version: current.version + 1, status: "active" })
          } else write({ ...current, version: current.version + 1, status: "unknown" })
        }
        throw error
      } finally { clearTimeout(deadline); if (active?.opId === opId) active = null }
    })
  }
  return { initialize, reconcile, observe, replace, logout, operation,
    current: () => observation,
    subscribe: (observer: (value: SessionObservation) => void) => { observers.add(observer); return () => observers.delete(observer) },
  }
}

function browserEnvironment(): SessionEnvironment | null {
  try {
    if (typeof window === "undefined" || !navigator.locks?.request || typeof crypto.getRandomValues !== "function") return null
    let channel: BroadcastChannel | undefined
    try { if (typeof BroadcastChannel !== "undefined") channel = new BroadcastChannel(SESSION_CHANNEL) } catch { /* storage events */ }
    return {
      storage: window.localStorage,
      locks: navigator.locks,
      id: () => newUuid(),
      notify: version => channel?.postMessage({ version }),
      listen: changed => {
        const storage = (event: StorageEvent) => { if (event.key === SESSION_KEY || event.key === null) changed() }
        const visibility = () => { if (document.visibilityState === "visible") changed() }
        channel?.addEventListener("message", changed)
        window.addEventListener("storage", storage)
        window.addEventListener("focus", changed)
        document.addEventListener("visibilitychange", visibility)
        window.addEventListener("pageshow", changed)
        return () => {
          channel?.removeEventListener("message", changed)
          window.removeEventListener("storage", storage)
          window.removeEventListener("focus", changed)
          document.removeEventListener("visibilitychange", visibility)
          window.removeEventListener("pageshow", changed)
        }
      },
    }
  } catch { return null }
}

export const sessionCoordinator = createSessionCoordinator(browserEnvironment())
