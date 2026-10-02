import { create } from "zustand"
import type { AppBrief, ProfileResp } from "@/api/types"

const TOKEN_KEY = "dante-token"
const APP_KEY = "dante-current-app"

interface AuthState {
  token: string | null
  profile: ProfileResp | null
  currentApplicationId: string | null
  contextEpoch: number
  contextSwitching: boolean
  beginContextSwitch: () => void
  endContextSwitch: () => void
  setToken: (token: string) => void
  setProfile: (profile: ProfileResp | null, epoch?: number) => void
  setCurrentApplication: (appId: string | null) => void
  /** 确保当前应用在可用列表内,返回有效应用 id */
  ensureApplication: (apps: AppBrief[]) => string | null
  logout: () => void
}

function readToken(): string | null {
  try {
    return localStorage.getItem(TOKEN_KEY)
  } catch {
    return null
  }
}

export const useAuthStore = create<AuthState>((set, get) => ({
  token: readToken(),
  profile: null,
  contextEpoch: 0,
  contextSwitching: false,
  currentApplicationId: (() => {
    try {
      return localStorage.getItem(APP_KEY)
    } catch {
      return null
    }
  })(),

  setToken: (token) => {
    try {
      localStorage.setItem(TOKEN_KEY, token)
    } catch {
      /* ignore */
    }
    set((state) => ({ token, profile: null, contextSwitching: false, contextEpoch: state.contextEpoch + 1 }))
  },

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

  logout: () => {
    try {
      localStorage.removeItem(TOKEN_KEY)
      localStorage.removeItem(APP_KEY)
    } catch {
      /* ignore */
    }
    set((state) => ({ token: null, profile: null, currentApplicationId: null,
      contextSwitching: false, contextEpoch: state.contextEpoch + 1 }))
  },
}))

/** 供非 React 环境(api 层)使用 */
export const authStore = useAuthStore
