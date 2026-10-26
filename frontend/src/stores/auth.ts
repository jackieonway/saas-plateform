import { create } from "zustand"
import type { AppBrief, ProfileResp } from "@/api/types"
import { bffSessionCoordinator } from "@/lib/bff-session"
import type { BffSessionView } from "@/lib/bff-session"
import { loadAuthenticationMode, type AuthenticationMode } from "@/lib/authentication-mode"
import type { SessionFence, SessionObservation, SessionStatus } from "@/lib/session-types"
import { contextIdentityFromResponse, contextIdentityKey, sameContextIdentity, type ContextIdentity } from "@/lib/context-identity"
export type { SessionFence } from "@/lib/session-types"

const APP_KEY_PREFIX = "platform-current-app-v2:"
let sessionLifetime = new AbortController()
let initialization: Promise<void> | null = null

interface AuthState {
  authMode: AuthenticationMode | null
  bffSession: BffSessionView | null
  tokenExpiresAt: number | null
  token: string | null
  profile: ProfileResp | null
  identity: ContextIdentity | null
  contextKey: string | null
  currentApplicationId: string | null
  contextEpoch: number
  contextSwitching: boolean
  sessionId: string | null
  sessionVersion: number
  sessionStatus: SessionStatus
  coordinationError: string | null
  initializeSessionCoordination: () => Promise<void>
  reconcileSession: () => Promise<void>
  beginContextSwitch: () => void
  endContextSwitch: () => void
  retireAuthorizationProjection: (expected?: SessionFence) => boolean
  setProfile: (profile: ProfileResp | null, epoch?: number) => void
  setCurrentApplication: (appId: string | null) => void
  /** 确保当前应用在可用列表内,返回有效应用 id */
  ensureApplication: (apps: AppBrief[]) => string | null
  logout: (expected?: SessionFence) => Promise<boolean>
}

export const useAuthStore = create<AuthState>((set, get) => ({
  authMode: null,
  bffSession: null,
  tokenExpiresAt: null,
  token: null,
  profile: null,
  identity: null,
  contextKey: null,
  contextEpoch: 0,
  contextSwitching: false,
  sessionId: null,
  sessionVersion: -1,
  sessionStatus: "initializing",
  coordinationError: null,
  initializeSessionCoordination: () => {
    initialization ??= (async () => {
      try {
        const authMode = await loadAuthenticationMode()
        set({ authMode })
        await bffSessionCoordinator.initialize()
      } catch (error) {
        set({ token: null, sessionStatus: "unavailable",
          coordinationError: error instanceof Error ? error.message : "认证服务暂时不可用，请重新检查" })
        throw error
      }
    })()
    return initialization
  },
  reconcileSession: async () => {
    if (get().authMode === "BFF") await bffSessionCoordinator.reconcile()
    else await get().initializeSessionCoordination()
  },
  currentApplicationId: null,

  beginContextSwitch: () => {
    sessionLifetime.abort()
    sessionLifetime = new AbortController()
    set((state) => ({ contextSwitching: true, profile: null, currentApplicationId: null, contextEpoch: state.contextEpoch + 1 }))
  },
  endContextSwitch: () => set({ contextSwitching: false }),

  retireAuthorizationProjection: (expected = captureSessionFence()) => {
    reconcileSessionSync()
    const state = get()
    if (state.sessionId !== expected.sessionId || state.contextKey !== expected.contextKey || !state.token || state.contextSwitching) return false
    sessionLifetime.abort()
    sessionLifetime = new AbortController()
    set({ profile: null, currentApplicationId: null, contextEpoch: state.contextEpoch + 1 })
    return true
  },

  setProfile: (profile, epoch) => {
    if (get().contextSwitching || (epoch !== undefined && get().contextEpoch !== epoch)) return
    if (profile && !sameContextIdentity(contextIdentityFromResponse(profile), get().identity)) {
      // A mismatched server response cannot replace the session's proven member.
      void get().logout(captureSessionFence()).catch(() => {})
      return
    }
    set({ profile })
  },

  setCurrentApplication: (appId) => {
    const state = get()
    if (!state.identity || !state.contextKey || state.contextSwitching || !state.token) return
    try {
      const key = `${APP_KEY_PREFIX}${state.contextKey}`
      if (appId) localStorage.setItem(key, appId)
      else localStorage.removeItem(key)
    } catch {
      /* ignore */
    }
    set({ currentApplicationId: appId })
  },

  ensureApplication: (apps) => {
    const { currentApplicationId, setCurrentApplication } = get()
    if (apps.length === 0) {
      if (currentApplicationId) setCurrentApplication(null)
      return null
    }
    const exists = apps.some((a) => a.id === currentApplicationId)
    if (exists) return currentApplicationId
    // 默认优先选 system 类应用,否则第一个
    const preferred = apps.find((a) => a.code === "system" || a.code.includes("system")) ?? apps[0]
    setCurrentApplication(preferred.id)
    return preferred.id
  },

  logout: (expected = captureSessionFence()) => bffSessionCoordinator.forget(expected),
}))

function applySharedSession({ record, status, error }: SessionObservation) {
  const state = useAuthStore.getState()
  const usable = status === "active" || status === "refreshing"
  const token = usable ? record?.token ?? null : null
  const sessionId = record?.sessionId ?? null
  const identity = record?.identity ?? null
  const contextKey = identity ? contextIdentityKey(identity) : null
  const switching = status === "switching" || status === "unknown" || status === "unavailable"
  const identityChanged = sessionId !== state.sessionId || contextKey !== state.contextKey
  const isolated = switching && !state.contextSwitching
  const resumed = state.contextSwitching && !switching && status === "active"
  const restoringPreference = state.sessionStatus === "initializing" && status === "active" && !!contextKey
  // A reload may restore only the preference stored under the complete proven
  // member namespace. Fresh login/switches never import an older global key.
  let restoredApplicationId: string | null = null
  if (restoringPreference && contextKey) {
    try { restoredApplicationId = localStorage.getItem(`${APP_KEY_PREFIX}${contextKey}`) } catch { /* Optional preference. */ }
  }
  if (state.token === token && state.sessionStatus === status &&
      state.sessionVersion === (record?.version ?? -1) && state.coordinationError === error) return
  if (identityChanged || isolated || resumed) {
    sessionLifetime.abort()
    sessionLifetime = new AbortController()
    try {
      if (state.contextKey) localStorage.removeItem(`${APP_KEY_PREFIX}${state.contextKey}`)
    } catch { /* Capability error is surfaced by the coordinator. */ }
  }
  useAuthStore.setState({ token, sessionId, identity, contextKey, sessionVersion: record?.version ?? -1,
    sessionStatus: status, coordinationError: error, contextSwitching: switching,
    ...((identityChanged || isolated || resumed) ? { profile: null, currentApplicationId: restoredApplicationId, contextEpoch: state.contextEpoch + 1 } : {}),
    ...(restoringPreference ? { currentApplicationId: restoredApplicationId } : {}),
  })
}
bffSessionCoordinator.subscribe(observation => {
  if (useAuthStore.getState().authMode !== "BFF") return
  applySharedSession(observation)
  useAuthStore.setState({ bffSession: observation.session, tokenExpiresAt: observation.tokenExpiresAt })
})

/** Reconcile before synchronous event callbacks (including notification delivery).
 * Reading a record never claims an abandoned operation: recovery requires Web Locks. */
export function reconcileSessionSync(): void {
  if (useAuthStore.getState().authMode === "BFF") bffSessionCoordinator.observe()
}
export function captureSessionSignal(): AbortSignal { return sessionLifetime.signal }

export function captureSessionFence(): SessionFence {
  const state = useAuthStore.getState()
  const fence = Object.freeze({ sessionId: state.sessionId, version: state.sessionVersion, contextKey: state.contextKey })
  reconcileSessionSync()
  return fence
}

/** Opt a captured platform bearer into request fencing. Credential tickets and
 * micro-application bearers deliberately do not use this entry point. */
export function capturePlatformSessionFence(expectedToken: string): SessionFence {
  reconcileSessionSync()
  const state = useAuthStore.getState()
  if (state.token !== expectedToken || (state.sessionStatus !== "active" && state.sessionStatus !== "refreshing")) {
    throw new DOMException("登录上下文已变化", "AbortError")
  }
  return Object.freeze({ sessionId: state.sessionId, version: state.sessionVersion, contextKey: state.contextKey })
}

/** 供非 React 环境(api 层)使用 */
export const authStore = useAuthStore
