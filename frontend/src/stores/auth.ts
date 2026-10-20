import { create } from "zustand"
import type { AppBrief, ProfileResp } from "@/api/types"
import { sessionCoordinator } from "@/lib/session-coordinator"
import type { SessionFence, SessionObservation, SessionStatus } from "@/lib/session-coordinator"
export type { SessionFence } from "@/lib/session-coordinator"

const APP_KEY = "platform-current-app"
let sessionLifetime = new AbortController()

interface AuthState {
  token: string | null
  refreshToken: string | null
  profile: ProfileResp | null
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
  setToken: (token: string) => Promise<boolean>
  setSession: (token: string, refreshToken?: string | null, expected?: SessionFence) => Promise<boolean>
  setProfile: (profile: ProfileResp | null, epoch?: number) => void
  setCurrentApplication: (appId: string | null) => void
  /** 确保当前应用在可用列表内,返回有效应用 id */
  ensureApplication: (apps: AppBrief[]) => string | null
  logout: (expected?: SessionFence) => Promise<boolean>
}

export const useAuthStore = create<AuthState>((set, get) => ({
  token: null,
  refreshToken: null,
  profile: null,
  contextEpoch: 0,
  contextSwitching: false,
  sessionId: null,
  sessionVersion: -1,
  sessionStatus: "initializing",
  coordinationError: null,
  initializeSessionCoordination: () => sessionCoordinator.initialize(),
  reconcileSession: async () => { await sessionCoordinator.reconcile() },
  currentApplicationId: (() => {
    try {
      return localStorage.getItem(APP_KEY)
    } catch {
      return null
    }
  })(),

  setToken: (token) => get().setSession(token),

  setSession: (token, refreshToken = null, expected = captureSessionFence()) => sessionCoordinator.replace({ token, refreshToken }, expected),

  beginContextSwitch: () => set((state) => ({
    contextSwitching: true, profile: null, contextEpoch: state.contextEpoch + 1,
  })),
  endContextSwitch: () => set({ contextSwitching: false }),

  setProfile: (profile, epoch) => {
    if (get().contextSwitching || (epoch !== undefined && get().contextEpoch !== epoch)) return
    set({ profile })
  },

  setCurrentApplication: (appId) => {
    try {
      if (appId) localStorage.setItem(APP_KEY, appId)
      else localStorage.removeItem(APP_KEY)
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

  logout: (expected = captureSessionFence()) => sessionCoordinator.logout(expected),
}))

function applySharedSession({ record, status, error }: SessionObservation) {
  const state = useAuthStore.getState()
  const usable = status === "active" || status === "refreshing"
  const token = usable ? record?.token ?? null : null
  const refreshToken = usable ? record?.refreshToken ?? null : null
  const sessionId = record?.sessionId ?? null
  const switching = status === "switching" || status === "unknown" || status === "unavailable"
  const identityChanged = sessionId !== state.sessionId
  const isolated = switching && !state.contextSwitching
  const resumed = state.contextSwitching && !switching && status === "active"
  if (state.token === token && state.refreshToken === refreshToken && state.sessionStatus === status &&
      state.sessionVersion === (record?.version ?? -1) && state.coordinationError === error) return
  if (identityChanged || isolated || resumed) {
    sessionLifetime.abort()
    sessionLifetime = new AbortController()
    try { localStorage.removeItem(APP_KEY) } catch { /* Capability error is surfaced by the coordinator. */ }
  }
  useAuthStore.setState({ token, refreshToken, sessionId, sessionVersion: record?.version ?? -1,
    sessionStatus: status, coordinationError: error, contextSwitching: switching,
    ...((identityChanged || isolated || resumed) ? { profile: null, currentApplicationId: null, contextEpoch: state.contextEpoch + 1 } : {}),
  })
}
sessionCoordinator.subscribe(applySharedSession)

/** Reconcile before synchronous event callbacks (including notification delivery).
 * Reading a record never claims an abandoned operation: recovery requires Web Locks. */
export function reconcileSessionSync(): void { sessionCoordinator.observe() }
export function captureSessionSignal(): AbortSignal { return sessionLifetime.signal }

export function captureSessionFence(): SessionFence {
  const state = useAuthStore.getState()
  const fence = Object.freeze({ sessionId: state.sessionId, version: state.sessionVersion })
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
  return Object.freeze({ sessionId: state.sessionId, version: state.sessionVersion })
}

/** 供非 React 环境(api 层)使用 */
export const authStore = useAuthStore
