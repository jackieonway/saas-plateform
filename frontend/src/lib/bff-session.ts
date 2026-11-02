import type { ContextResp, Membership } from "@/api/types"
import { canSelectConsoleMembership, contextIdentityFromResponse, contextIdentityKey, isContextIdentity, sameContextIdentity } from "@/lib/context-identity"
import type { ContextIdentity } from "@/lib/context-identity"
import type { SessionFence, SessionObservation, SessionRecord, SessionStatus } from "@/lib/session-types"

/** Public account/context projection. Authentication material never enters this view. */
export interface BffSessionView {
  sessionId: string
  version: string
  account: { id: string; username: string; realName: string; avatar: string }
  memberships: Membership[]
  context: ContextIdentity | null
  expiresAt: string
  centralizedContextSelection?: boolean
}

export interface BffObservation extends SessionObservation {
  session: BffSessionView | null
  tokenExpiresAt: number | null
}

export interface BffSessionNotice {
  sessionId: string | null
  revision: string | null
  invalid: boolean
  phase?: "switching" | "settled"
  switchId?: string
}

interface SwitchCorrelation { sessionId: string; revision: string; switchId: string }
interface PendingSwitch extends SwitchCorrelation { local: boolean; unknown: boolean }

export interface BffSessionEnvironment {
  navigate?: (url: string) => void
  fetch: typeof fetch
  now?: () => number
  setTimeout?: (callback: () => void, milliseconds: number) => unknown
  clearTimeout?: (handle: unknown) => void
  storage?: Pick<Storage, "removeItem">
  returnedSwitch?: SwitchCorrelation | null
  notify?: (notice: BffSessionNotice) => void
  listen?: (invalidated: (notice?: BffSessionNotice) => void, focused: () => void, suspended: () => void) => (() => void)
}

export class BffSessionError extends Error {
  constructor(message: string, readonly code: string, readonly status = 0) {
    super(message)
    this.name = code === "SESSION_CHANGED" ? "AbortError" : "BffSessionError"
  }
}

const credentialKeys = ["platform-session-v2", "platform-session-v1", "platform-token", "platform-refresh-token", "platform-remember-username", "platform-current-app"]
const switchReturnParameter = "__platform_context_switch"
const switchIdPattern = /^[0-9a-f]{32}$/
const maximumPendingSwitches = 32
const maximumSettledSwitches = 128
const uuid7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
const i64Maximum = "9223372036854775807"
const unavailableMessage = "无法确认统一登录状态，平台请求已暂停，请稍后重试或重新登录。"
const changed = () => new BffSessionError("登录上下文已变化", "SESSION_CHANGED")
const invalidResponse = () => new BffSessionError("统一登录响应无效，平台请求已暂停。", "SESSION_UNKNOWN", 502)
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value)
const decimal = (value: unknown): value is string => typeof value === "string" && /^(0|[1-9][0-9]*)$/.test(value)
  && (value.length < i64Maximum.length || value.length === i64Maximum.length && value <= i64Maximum)
const hasSecretResponse = (value: Record<string, unknown>) => ["refreshToken", "refresh_token", "refreshExpiresIn", "idToken", "id_token", "access_token"].some(key => Object.hasOwn(value, key))
const copyIdentity = (value: ContextIdentity): ContextIdentity => Object.freeze({ userId: value.userId, tenantId: value.tenantId,
  organizationId: value.organizationId, membershipId: value.membershipId, memberType: value.memberType,
  identityVersion: value.identityVersion, accountVersion: value.accountVersion })

function correlatedSwitch(value: unknown): value is SwitchCorrelation {
  return object(value) && Object.keys(value).length === 3
    && typeof value.sessionId === "string" && uuid7.test(value.sessionId)
    && decimal(value.revision) && typeof value.switchId === "string" && switchIdPattern.test(value.switchId)
}

function switchReturnPath(path: string, operation: SwitchCorrelation): string {
  const invalid = () => new BffSessionError("切换返回路径无效。", "INVALID_CONTEXT_RETURN", 400)
  let target: URL
  try { target = new URL(path, "https://bff.invalid") } catch { throw invalid() }
  if (!path.startsWith("/") || path.startsWith("//") || path.includes("\\") || [...path].some(char => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127)
      || target.origin !== "https://bff.invalid" || target.hash || target.searchParams.has(switchReturnParameter)) throw invalid()
  target.searchParams.set(switchReturnParameter, `${operation.sessionId}.${operation.revision}.${operation.switchId}`)
  const result = target.pathname + target.search
  if (result.length > 2048) throw invalid()
  return result
}

/** The return marker carries correlation only. Consume it before private pages,
 * requests or caches observe the URL; a live GET and bridge still own authority. */
function consumeSwitchReturn(): SwitchCorrelation | null {
  try {
    const url = new URL(window.location.href)
    const markers = url.searchParams.getAll(switchReturnParameter)
    if (!markers.length) return null
    url.searchParams.delete(switchReturnParameter)
    window.history.replaceState(window.history.state, "", url.pathname + url.search + url.hash)
    if (markers.length !== 1 || markers[0].length > 90) return null
    const [sessionId, revision, switchId, extra] = markers[0].split(".")
    const operation = { sessionId, revision, switchId }
    return extra === undefined && correlatedSwitch(operation) ? operation : null
  } catch { return null }
}

function membership(value: unknown, accountId: string): Membership {
  if (!object(value) || value.userId !== accountId || ![value.userId, value.tenantId, value.organizationId, value.membershipId].every(id => typeof id === "string" && uuid7.test(id))
    || value.memberType !== "EMPLOYEE" || value.status !== "ENABLED"
    || !decimal(value.identityVersion) || value.identityVersion === "0" || typeof value.consoleSelectable !== "boolean"
    || value.consoleSelectable !== true || value.selectionDisabledReason !== null
    || !["organizationName", "organizationPath", "tenantName"].every(key => typeof value[key] === "string")) throw invalidResponse()
  const row: Membership = {
    userId: accountId, tenantId: value.tenantId as string, organizationId: value.organizationId as string,
    membershipId: value.membershipId as string, memberType: value.memberType as Membership["memberType"],
    identityVersion: value.identityVersion, status: value.status as Membership["status"],
    organizationName: value.organizationName as string, organizationPath: value.organizationPath as string,
    tenantName: value.tenantName as string, consoleSelectable: value.consoleSelectable,
    selectionDisabledReason: value.selectionDisabledReason as Membership["selectionDisabledReason"],
  }
  for (const key of ["username", "realName", "departmentName"] as const) {
    if (value[key] !== undefined && value[key] !== null) {
      if (typeof value[key] !== "string") throw invalidResponse()
      row[key] = value[key]
    }
  }
  if (value.departmentId !== undefined && value.departmentId !== null) {
    if (typeof value.departmentId !== "string" || !uuid7.test(value.departmentId)) throw invalidResponse()
    row.departmentId = value.departmentId
  }
  return Object.freeze(row)
}

function memberMatches(view: BffSessionView, identity: ContextIdentity): boolean {
  return identity.userId === view.account.id && identity.memberType === "EMPLOYEE" && view.memberships.some(row =>
    canSelectConsoleMembership(row) && row.userId === identity.userId && row.tenantId === identity.tenantId
    && row.organizationId === identity.organizationId && row.membershipId === identity.membershipId && row.identityVersion === identity.identityVersion)
}

function sessionResponse(value: unknown, now: number): { view: BffSessionView; csrf: string } {
  if (!object(value) || hasSecretResponse(value) || Object.hasOwn(value, "accessToken") || typeof value.sessionId !== "string" || !uuid7.test(value.sessionId)
    || !decimal(value.version) || !object(value.account) || !uuid7.test(String(value.account.id))
    || !["username", "realName", "avatar"].every(key => typeof (value.account as Record<string, unknown>)[key] === "string")
    || !Array.isArray(value.memberships) || typeof value.expiresAt !== "string" || !Number.isFinite(Date.parse(value.expiresAt))
    || typeof value.csrfToken !== "string" || !/^[0-9a-f]{64}$/.test(value.csrfToken)) throw invalidResponse()
  if (Date.parse(value.expiresAt) <= now) throw new BffSessionError("统一登录已过期，请重新登录。", "SESSION_AUTHENTICATION_REQUIRED", 401)
  const account = Object.freeze({ id: value.account.id as string, username: value.account.username as string,
    realName: value.account.realName as string, avatar: value.account.avatar as string })
  const view: BffSessionView = { centralizedContextSelection: value.centralizedContextSelection === true, sessionId: value.sessionId, version: value.version, account,
    memberships: value.memberships.map(row => membership(row, account.id)), context: null, expiresAt: value.expiresAt }
  if (value.context !== null) {
    if (!isContextIdentity(value.context) || !memberMatches(view, value.context)) throw invalidResponse()
    view.context = copyIdentity(value.context)
  }
  return { view: Object.freeze(view), csrf: value.csrfToken }
}

function contextResponse(value: unknown, view: BffSessionView, selecting?: { organizationId: string; membershipId: string }): { data: ContextResp; identity: ContextIdentity; version: string } {
  if (!object(value) || hasSecretResponse(value) || value.sessionId !== view.sessionId || !decimal(value.version)
    || typeof value.accessToken !== "string" || !value.accessToken || !Number.isInteger(value.expiresIn)
    || (value.expiresIn as number) < 1 || (value.expiresIn as number) > 300 || !object(value.user) || !object(value.tenant) || !object(value.organization)) throw invalidResponse()
  const identity = contextIdentityFromResponse(value)
  if (!identity || !memberMatches(view, identity)) throw invalidResponse()
  if (selecting) {
    if (identity.organizationId !== selecting.organizationId || identity.membershipId !== selecting.membershipId
      || BigInt(value.version) !== BigInt(view.version) + 1n) throw invalidResponse()
  } else if (value.version !== view.version || !sameContextIdentity(view.context, identity)) throw invalidResponse()
  if (!["username", "realName", "email", "phone", "avatar", "gender"].every(key => typeof (value.user as Record<string, unknown>)[key] === "string")
    || typeof value.user.isSuperAdmin !== "boolean" || typeof value.tenant.name !== "string" || typeof value.tenant.code !== "string"
    || typeof value.organization.name !== "string" || typeof value.organization.path !== "string"
    || value.platformContext !== undefined && typeof value.platformContext !== "boolean") throw invalidResponse()
  const data: ContextResp = { accessToken: value.accessToken, expiresIn: value.expiresIn as number,
    user: { id: identity.userId, username: value.user.username as string, realName: value.user.realName as string,
      email: value.user.email as string, phone: value.user.phone as string, avatar: value.user.avatar as string,
      gender: value.user.gender as string, isSuperAdmin: value.user.isSuperAdmin },
    tenant: { id: identity.tenantId, name: value.tenant.name, code: value.tenant.code },
    organization: { id: identity.organizationId, name: value.organization.name, path: value.organization.path },
    membershipId: identity.membershipId, memberType: identity.memberType, identityVersion: identity.identityVersion,
    accountVersion: identity.accountVersion, ...(typeof value.platformContext === "boolean" ? { platformContext: value.platformContext } : {}),
  }
  if (typeof value.tenant.isPlatform === "boolean") data.tenant.isPlatform = value.tenant.isPlatform
  return { data, identity, version: value.version }
}

/** No storage or Web Locks are needed: the Cookie and server CAS own authority. */
export function createBffSessionCoordinator(environment: BffSessionEnvironment | null = browserEnvironment()) {
  let observation: BffObservation = { record: null, status: "initializing", error: null, session: null, tokenExpiresAt: null }
  const observers = new Set<(value: BffObservation) => void>()
  const controllers = new Set<AbortController>()
  const now = environment?.now ?? Date.now
  const schedule = environment?.setTimeout ?? ((callback: () => void, milliseconds: number) => setTimeout(callback, milliseconds))
  const cancel = environment?.clearTimeout ?? ((handle: unknown) => clearTimeout(handle as ReturnType<typeof setTimeout>))
  let csrf: string | null = null
  let generation = 0
  let localVersion = 0
  let disposed = false
  let initialized = false
  let needsRecovery = true
  let unsubscribe: (() => void) | undefined
  let renewalTimer: unknown = null
  let expiryTimer: unknown = null
  let recovery: { generation: number; promise: Promise<void> } | null = null
  let renewing: { sessionId: string | null; contextKey: string | null; promise: Promise<string> } | null = null
  let mutationTail: Promise<unknown> = Promise.resolve()
  const pendingSwitches = new Map<string, PendingSwitch>()
  const settledSwitches = new Set<string>()
  let switchOverflow: { sessionId: string; revision: string; ambiguous: boolean } | null = null
  let localSwitch = correlatedSwitch(environment?.returnedSwitch) ? { ...environment.returnedSwitch } : null
  // Includes an UNKNOWN POST outcome until a newer authority or explicit action.
  let localSwitchInFlight = false
  let announcedSwitch = localSwitch !== null
  const revisions = new Map<string, string>()

  const switchKey = (operation: SwitchCorrelation) => `${operation.sessionId}:${operation.revision}:${operation.switchId}`
  const blockedAt = (operation: Pick<SwitchCorrelation, "sessionId" | "revision">, view: BffSessionView) =>
    view.sessionId === operation.sessionId && BigInt(view.version) <= BigInt(operation.revision)
  const overflowBlocked = (view: BffSessionView) => !!switchOverflow && (switchOverflow.ambiguous || blockedAt(switchOverflow, view))
  function rememberSettled(key: string) {
    settledSwitches.add(key)
    if (settledSwitches.size > maximumSettledSwitches) settledSwitches.delete(settledSwitches.values().next().value!)
  }
  function finishSwitch(operation: SwitchCorrelation) {
    const key = switchKey(operation)
    pendingSwitches.delete(key); rememberSettled(key)
  }
  function trackSwitch(operation: SwitchCorrelation, local: boolean) {
    const key = switchKey(operation)
    if (settledSwitches.has(key) || pendingSwitches.has(key)) return
    if (pendingSwitches.size >= maximumPendingSwitches) {
      // Never evict an unresolved operation. Overflow remains paused until an
      // authoritative new source/version (or explicit reauthentication).
      if (!switchOverflow) switchOverflow = { sessionId: operation.sessionId, revision: operation.revision, ambiguous: false }
      else if (switchOverflow.sessionId !== operation.sessionId) switchOverflow.ambiguous = true
      else if (BigInt(operation.revision) > BigInt(switchOverflow.revision)) switchOverflow.revision = operation.revision
      return
    }
    pendingSwitches.set(key, { ...operation, local, unknown: false })
  }
  function switchBlocked(view: BffSessionView): boolean {
    return [...pendingSwitches.values()].some(operation => blockedAt(operation, view))
      || overflowBlocked(view)
      || localSwitchInFlight && !!localSwitch && blockedAt(localSwitch, view)
  }
  function publishSwitchPause(view: BffSessionView) {
    const unknown = overflowBlocked(view)
      || [...pendingSwitches.values()].some(operation => operation.local && operation.unknown && blockedAt(operation, view))
    publish(unknown ? "unknown" : "switching", view, null, null, unknown ? unavailableMessage : null)
  }

  function clearTimers() {
    if (renewalTimer !== null) cancel(renewalTimer)
    if (expiryTimer !== null) cancel(expiryTimer)
    renewalTimer = null; expiryTimer = null
  }
  function timer(callback: () => void, delay: number): unknown {
    const handle = schedule(callback, Math.max(1, Math.min(2_147_483_647, delay)))
    if (handle && typeof handle === "object" && "unref" in handle) (handle as { unref?: () => void }).unref?.()
    return handle
  }
  function publish(status: SessionStatus, view = observation.session, token: string | null = null, expires: number | null = null, error: string | null = null) {
    if (disposed) return
    const identity = view?.context ?? null
    const record: SessionRecord = { version: ++localVersion,
      sessionId: view ? `${view.sessionId}:${view.version}` : null,
      status: status === "anonymous" ? "anonymous" : status === "unknown" || status === "unavailable" ? "unknown" : status === "switching" || status === "refreshing" ? "in-flight" : "active",
      token, identity, contextKey: identity ? contextIdentityKey(identity) : null }
    observation = { record, status, error, session: view, tokenExpiresAt: expires }
    observers.forEach(observer => observer(observation))
  }
  function fence(): SessionFence {
    return { sessionId: observation.record?.sessionId ?? null, version: observation.record?.version ?? -1, contextKey: observation.record?.contextKey ?? null }
  }
  function matches(expected: SessionFence): boolean {
    const current = fence()
    return current.sessionId === expected.sessionId && current.contextKey === expected.contextKey
      && (expected.sessionId !== null || current.version === expected.version)
  }
  function requireFence(expected: SessionFence) { if (disposed || !matches(expected)) throw changed() }
  function checkGeneration(expected: number) { if (disposed || expected !== generation) throw changed() }
  function notify(view: BffSessionView | null, invalid: boolean, phase: "switching" | "settled" = "settled") {
    announcedSwitch = phase === "switching"
    const operation = localSwitch
    try { environment?.notify?.({ sessionId: operation?.sessionId ?? view?.sessionId ?? null,
      revision: operation?.revision ?? view?.version ?? null, invalid,
      ...(operation ? { phase, switchId: operation.switchId } : {}) }) } catch { /* Authority remains on the server. */ }
    if (phase === "settled") {
      if (operation) finishSwitch(operation)
      localSwitch = null; localSwitchInFlight = false
    }
  }
  function newSwitch(view: BffSessionView): SwitchCorrelation {
    let switchId: string
    try { switchId = [...crypto.getRandomValues(new Uint8Array(16))].map(byte => byte.toString(16).padStart(2, "0")).join("") }
    catch { throw new BffSessionError(unavailableMessage, "SESSION_UNKNOWN", 503) }
    return { sessionId: view.sessionId, revision: view.version, switchId }
  }
  function startSwitch(operation: SwitchCorrelation) {
    trackSwitch(operation, true)
    localSwitch = operation; localSwitchInFlight = true
  }
  function pause() {
    clearTimers(); needsRecovery = true
    publish("unknown", observation.session, null, null, unavailableMessage)
  }
  function retire() {
    ++generation
    controllers.forEach(controller => controller.abort())
    renewing = null
    clearTimers(); needsRecovery = true
    publish("unknown", observation.session, null, null, unavailableMessage)
  }
  function anonymous(announce = false, settleOperation = true) {
    const previous = observation.session
    const settleSwitch = announcedSwitch
    ++generation
    controllers.forEach(controller => controller.abort())
    clearTimers(); csrf = null; needsRecovery = false; pendingSwitches.clear(); switchOverflow = null
    publish("anonymous", null)
    if (announce || settleSwitch && settleOperation) notify(previous, true)
    else { localSwitch = null; localSwitchInFlight = false; announcedSwitch = false }
  }
  function armTimers() {
    clearTimers()
    const expiry = observation.tokenExpiresAt
    if (expiry === null || !observation.record?.token) return
    const expected = fence(), ttl = expiry - now()
    expiryTimer = timer(() => {
      if (matches(expected) && observation.tokenExpiresAt === expiry && now() >= expiry) pause()
    }, ttl)
    renewalTimer = timer(() => {
      if (matches(expected)) void renew(expected).catch(() => {})
    }, ttl - Math.min(30_000, Math.max(1_000, ttl / 5)))
  }
  async function request(path: string, ticket: number, body?: Record<string, unknown>): Promise<unknown> {
    checkGeneration(ticket)
    if (!environment) throw new BffSessionError(unavailableMessage, "SESSION_UNKNOWN", 503)
    if (body && !csrf) throw invalidResponse()
    const controller = new AbortController()
    controllers.add(controller)
    let abortRequest: (() => void) | undefined
    const aborted = new Promise<never>((_resolve, reject) => {
      abortRequest = () => reject(new BffSessionError(unavailableMessage, "SESSION_UNKNOWN", 503))
      controller.signal.addEventListener("abort", abortRequest, { once: true })
    })
    const timeout = timer(() => controller.abort(), 20_000)
    try {
      // Settle the queue even when a transport or body reader ignores abort.
      const response = async () => {
        const result = await environment.fetch(path, { method: body ? "POST" : "GET", credentials: "same-origin", cache: "no-store", redirect: "error",
          headers: { Accept: "application/json", ...(body ? { "Content-Type": "application/json", "X-CSRF-Token": csrf! } : {}) },
          ...(body ? { body: JSON.stringify(body) } : {}), signal: controller.signal })
        checkGeneration(ticket)
        if (controller.signal.aborted) throw new BffSessionError(unavailableMessage, "SESSION_UNKNOWN", 503)
        if (!result.ok) throw new BffSessionError(result.status === 401 ? "统一登录已失效，请重新登录。" : unavailableMessage,
          result.status === 401 ? "SESSION_AUTHENTICATION_REQUIRED" : "BFF_REQUEST_FAILED", result.status)
        const data: unknown = await result.json()
        checkGeneration(ticket)
        if (controller.signal.aborted) throw new BffSessionError(unavailableMessage, "SESSION_UNKNOWN", 503)
        return data
      }
      return await Promise.race([response(), aborted])
    } catch (error) {
      checkGeneration(ticket)
      if (error instanceof BffSessionError) throw error
      throw new BffSessionError(unavailableMessage, "SESSION_UNKNOWN", 503)
    } finally {
      cancel(timeout); controllers.delete(controller)
      if (abortRequest) controller.signal.removeEventListener("abort", abortRequest)
    }
  }
  function accept(view: BffSessionView, token: string | null, expiry: number | null) {
    const previousRevision = revisions.get(view.sessionId)
    if (previousRevision !== undefined && BigInt(view.version) < BigInt(previousRevision)) throw invalidResponse()
    revisions.set(view.sessionId, view.version)
    for (const [key, operation] of pendingSwitches) {
      if (!blockedAt(operation, view)) { pendingSwitches.delete(key); rememberSettled(key) }
    }
    if (switchOverflow && !overflowBlocked(view)) switchOverflow = null
    if (switchBlocked(view)) { needsRecovery = true; publishSwitchPause(view); return }
    needsRecovery = false
    publish("active", view, token, expiry)
    armTimers()
  }
  async function bridge(view: BffSessionView, ticket: number, selecting?: { organizationId: string; membershipId: string }): Promise<ContextResp> {
    const started = now()
    const raw = await request("/auth/context-token", ticket, { expectedVersion: view.version, ...selecting })
    const checked = contextResponse(raw, view, selecting)
    const expiry = Math.min(started + checked.data.expiresIn * 1000, Date.parse(view.expiresAt))
    if (expiry <= now()) throw invalidResponse()
    accept(Object.freeze({ ...view, version: checked.version, context: checked.identity }), checked.data.accessToken, expiry)
    if (selecting) notify(observation.session, false)
    return checked.data
  }
  async function restore(ticket: number) {
    try {
      const raw = await request("/auth/session", ticket)
      const parsed = sessionResponse(raw, now())
      // A switching notice freezes this revision. A read that raced ahead of
      // server freeze cannot restore it. Only a terminal notice or a newer
      // authoritative revision releases the barrier; no credentials are shared.
      if (switchBlocked(parsed.view)) {
        // A definitive local rejection or correlated return may settle only
        // itself after this authenticated source GET. Other operations keep
        // the UI frozen, and no bearer bridge runs until their barriers clear.
        if (localSwitch && !localSwitchInFlight && announcedSwitch) notify(parsed.view, false)
        publishSwitchPause(parsed.view)
        return
      }
      const previous = observation.session, token = observation.record?.token, expiry = observation.tokenExpiresAt
      const centralChanged = parsed.view.centralizedContextSelection && (previous?.sessionId !== parsed.view.sessionId
        || previous.version !== parsed.view.version || !sameContextIdentity(previous.context, parsed.view.context))
      const reusable = !needsRecovery && token && expiry !== null && expiry > now()
        && previous?.sessionId === parsed.view.sessionId && previous.version === parsed.view.version
        && sameContextIdentity(previous.context, parsed.view.context)
      csrf = parsed.csrf
      if (!parsed.view.context) { accept(parsed.view, null, null); if (centralChanged || announcedSwitch) notify(observation.session, false); return }
      if (reusable && typeof token === "string" && expiry !== null) { accept(parsed.view, token, expiry); return }
      // Do not let a previous context stay usable during a replacement proof read.
      publish(observation.status === "initializing" ? "initializing" : "refreshing", parsed.view)
      await bridge(parsed.view, ticket)
      if (centralChanged || announcedSwitch) {
        notify(observation.session, false)
      }
    } catch (error) {
      checkGeneration(ticket)
      if (error instanceof BffSessionError && error.status === 401) { anonymous(); return }
      pause()
      throw error
    }
  }
  async function reconcile(): Promise<void> {
    await mutationTail
    if (disposed) throw changed()
    const ticket = generation
    if (recovery?.generation === ticket) return recovery.promise
    const entry = { generation: ticket, promise: Promise.resolve() }
    entry.promise = restore(ticket)
    recovery = entry
    try { await entry.promise } finally { if (recovery === entry) recovery = null }
  }
  function observe(): void {
    if (!disposed && observation.tokenExpiresAt !== null && observation.tokenExpiresAt <= now()) {
      pause()
      void reconcile().catch(() => {})
    }
  }
  async function initialize(): Promise<void> {
    if (!initialized) {
      initialized = true
      for (const key of credentialKeys) { try { environment?.storage?.removeItem(key) } catch { /* Optional cleanup never owns BFF authority. */ } }
      unsubscribe = environment?.listen?.((notice) => {
        const current = observation.session
        if (notice?.phase === "switching" && notice.sessionId && current && notice.sessionId !== current.sessionId) return
        if (notice?.sessionId === current?.sessionId && notice?.revision && current && BigInt(notice.revision) < BigInt(current.version)) return
        if (notice?.phase === "switching") {
          if (!notice.sessionId || !notice.revision || !notice.switchId || !switchIdPattern.test(notice.switchId)) return
          const operation = { sessionId: notice.sessionId, revision: notice.revision, switchId: notice.switchId }
          if (settledSwitches.has(switchKey(operation))) return
          trackSwitch(operation, false)
          // Record simultaneous foreign switches without aborting this tab's POST.
          if (localSwitchInFlight) { void reconcile().catch(() => {}); return }
          retire(); publish("switching", observation.session)
          return
        }
        // A terminal notice is only a hint for this exact switch revision.
        // Unrelated or unbound notices still recheck authority, but cannot
        // release a barrier while GET may still return the original source.
        if (notice?.sessionId && notice.revision && notice.switchId && switchIdPattern.test(notice.switchId)) {
          const operation = { sessionId: notice.sessionId, revision: notice.revision, switchId: notice.switchId }
          if (!pendingSwitches.get(switchKey(operation))?.local) finishSwitch(operation)
        }
        // Even a copied matching hint cannot settle this tab's active/UNKNOWN POST.
        if (localSwitchInFlight) { void reconcile().catch(() => {}); return }
        retire(); void reconcile().catch(() => {})
      }, () => { observe(); void reconcile().catch(() => {}) }, () => {
        retire(); csrf = null
      })
    }
    await reconcile()
  }
  function enqueue<T>(action: () => Promise<T>): Promise<T> {
    const result = mutationTail.then(action, action)
    mutationTail = result.catch(() => {})
    return result
  }
  async function mutation<T>(expected: SessionFence, action: (view: BffSessionView, ticket: number) => Promise<T>): Promise<T> {
    requireFence(expected)
    if (needsRecovery) { await restore(generation); requireFence(expected) }
    const view = observation.session
    if (!view || Date.parse(view.expiresAt) <= now()) { anonymous(); throw new BffSessionError("统一登录已失效，请重新登录。", "SESSION_AUTHENTICATION_REQUIRED", 401) }
    // Every remote mutation, including an already queued renew, must stop
    // after recovery if any operation still freezes this exact source.
    // Explicit reauthentication navigates outside this credential coordinator.
    if (switchBlocked(view)) throw changed()
    // Fence out a GET that started before this mutation, even if fetch ignores abort.
    ++generation; controllers.forEach(controller => controller.abort())
    const ticket = generation
    try { return await action(view, ticket) }
    catch (error) {
      checkGeneration(ticket)
      // A failed transport or 5xx may follow server-side creation. Keep the
      // original correlation barrier: another old GET cannot settle UNKNOWN.
      if (error instanceof BffSessionError && error.status >= 400 && error.status < 500) {
        localSwitchInFlight = false
        if (localSwitch) pendingSwitches.delete(switchKey(localSwitch))
      } else if (localSwitch) {
        const operation = pendingSwitches.get(switchKey(localSwitch))
        if (operation) operation.unknown = true
      }
      if (error instanceof BffSessionError && error.status === 401) anonymous()
      else if (error instanceof BffSessionError && error.status === 409) {
        pause()
        await restore(ticket)
        throw changed()
      } else pause()
      throw error
    }
  }
  function renew(expected: SessionFence): Promise<string> {
    if (renewing?.sessionId === expected.sessionId && renewing.contextKey === expected.contextKey) return renewing.promise
    const entry = { sessionId: expected.sessionId, contextKey: expected.contextKey, promise: Promise.resolve("") }
    entry.promise = enqueue(() => {
      const recovering = needsRecovery
      return mutation(expected, async (view, ticket) => {
        if (!view.context) throw new BffSessionError("请选择有效员工身份。", "BFF_CONTEXT_REQUIRED", 400)
        if (recovering && observation.record?.token && observation.tokenExpiresAt !== null && observation.tokenExpiresAt > now()) return observation.record.token
        publish("refreshing", view, observation.record?.token ?? null, observation.tokenExpiresAt)
        return (await bridge(view, ticket)).accessToken
      })
    })
    renewing = entry
    void entry.promise.finally(() => { if (renewing === entry) renewing = null }).catch(() => {})
    return entry.promise
  }
  function selectContext(organizationId: string, membershipId: string, expected: SessionFence): Promise<ContextResp> {
    return enqueue(() => mutation(expected, async (view, ticket) => {
      if (!uuid7.test(organizationId) || !uuid7.test(membershipId) || !view.memberships.some(row => canSelectConsoleMembership(row)
        && row.organizationId === organizationId && row.membershipId === membershipId)) throw new BffSessionError("请选择有效的具体员工身份。", "INVALID_CONTEXT_SELECTION", 400)
      startSwitch(newSwitch(view))
      clearTimers(); publish("switching", view)
      notify(view, false, "switching")
      return bridge(view, ticket, { organizationId, membershipId })
    }))
  }
  function beginContextSwitch(expected: SessionFence, returnPath: string): Promise<void> {
    return enqueue(() => mutation(expected, async (view, ticket) => {
      if (!view.centralizedContextSelection) throw new BffSessionError("统一身份选择尚未启用", "CENTRAL_SELECTION_REQUIRED", 409)
      const operation = newSwitch(view)
      const correlatedReturn = switchReturnPath(returnPath, operation)
      startSwitch(operation)
      clearTimers(); publish("switching", view)
      notify(view, false, "switching")
      const data = await request("/auth/context-switch", ticket, { expectedVersion: view.version, returnPath: correlatedReturn })
      if (!object(data) || typeof data.redirectTo !== "string") throw invalidResponse()
      const target = new URL(data.redirectTo)
      if (target.protocol !== "https:" || target.username || target.password || target.hash || target.pathname !== "/oauth/authorize" || !environment?.navigate) throw invalidResponse()
      environment.navigate(target.href)
    }))
  }
  function logout(expected: SessionFence, scope: "PLATFORM" | "GLOBAL" = "PLATFORM"): Promise<boolean> {
    return enqueue(async () => {
      if (!matches(expected)) return false
      return mutation(expected, async (view, ticket) => {
        if (scope !== "PLATFORM" && scope !== "GLOBAL") throw new BffSessionError("退出范围无效。", "INVALID_LOGOUT_SCOPE", 400)
        clearTimers(); publish("switching", view)
        const data = await request("/auth/logout", ticket, { expectedVersion: view.version, scope })
        if (!object(data) || data.redirectTo !== "/login?signedOut=1") throw invalidResponse()
        anonymous(true)
        return true
      })
    })
  }
  async function forget(expected: SessionFence): Promise<boolean> {
    if (!matches(expected) || disposed) return false
    // Local credential removal proves no outcome for a pending remote switch.
    anonymous(false, false)
    return true
  }
  function dispose() {
    if (disposed) return
    disposed = true; ++generation
    controllers.forEach(controller => controller.abort()); clearTimers(); unsubscribe?.()
    controllers.clear(); observers.clear(); csrf = null
    observation = { record: null, status: "anonymous", error: null, session: null, tokenExpiresAt: null }
  }
  return { initialize, reconcile, observe, current: (): BffObservation => observation,
    subscribe: (observer: (value: BffObservation) => void) => { observers.add(observer); return () => { observers.delete(observer) } },
    renew, selectContext, beginContextSwitch, logout, forget, dispose }
}

function browserEnvironment(): BffSessionEnvironment | null {
  if (typeof window === "undefined" || typeof fetch !== "function") return null
  let channel: BroadcastChannel | undefined
  try { if (typeof BroadcastChannel !== "undefined") channel = new BroadcastChannel("platform-bff-session-v1") } catch { /* Focus and each request still read authority. */ }
  return {
    navigate: url => window.location.assign(url),
    fetch: (...args) => fetch(...args),
    get storage() { try { return window.localStorage } catch { return undefined } },
    returnedSwitch: consumeSwitchReturn(),
    notify: notice => channel?.postMessage(notice),
    listen: (invalidated, focused, suspended) => {
      const message = (event: MessageEvent<unknown>) => {
        if (!object(event.data) || ![3,4,5].includes(Object.keys(event.data).length) || typeof event.data.invalid !== "boolean"
          || Object.keys(event.data).some(key => !["sessionId","revision","invalid","phase","switchId"].includes(key))
          || event.data.phase !== undefined && event.data.phase !== "switching" && event.data.phase !== "settled"
          || event.data.switchId !== undefined && (typeof event.data.switchId !== "string" || !switchIdPattern.test(event.data.switchId))
          || event.data.phase === "switching" && event.data.switchId === undefined
          || !(event.data.sessionId === null || typeof event.data.sessionId === "string" && uuid7.test(event.data.sessionId))
          || !(event.data.revision === null || decimal(event.data.revision))) return
        invalidated(event.data as unknown as BffSessionNotice)
      }
      const visible = () => { if (document.visibilityState === "visible") focused() }
      channel?.addEventListener("message", message)
      window.addEventListener("focus", focused); window.addEventListener("pageshow", focused); window.addEventListener("pagehide", suspended)
      document.addEventListener("visibilitychange", visible)
      return () => {
        channel?.removeEventListener("message", message); channel?.close()
        window.removeEventListener("focus", focused); window.removeEventListener("pageshow", focused); window.removeEventListener("pagehide", suspended)
        document.removeEventListener("visibilitychange", visible)
      }
    },
  }
}

export const bffSessionCoordinator = createBffSessionCoordinator()
