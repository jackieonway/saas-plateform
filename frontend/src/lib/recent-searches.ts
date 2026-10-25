import { useAuthStore } from "@/stores/auth"
import { contextIdentityFromResponse, contextIdentityKey, isContextIdentity, sameContextIdentity } from "@/lib/context-identity"

const MAX_RECENT_SEARCHES = 6

function storageKey(expectedContextKey: string | null): string | null {
  const state = useAuthStore.getState()
  if (!expectedContextKey || state.contextSwitching || !state.token ||
      state.contextKey !== expectedContextKey || !isContextIdentity(state.identity) ||
      contextIdentityKey(state.identity) !== expectedContextKey ||
      !sameContextIdentity(contextIdentityFromResponse(state.profile), state.identity)) return null
  return `platform-recent-searches-v2:${expectedContextKey}`
}

export function readRecentSearches(expectedContextKey: string | null): string[] {
  const key = storageKey(expectedContextKey)
  if (!key) return []
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(key) ?? "[]")
    return Array.isArray(parsed) ? parsed.filter((term): term is string => typeof term === "string").slice(0, MAX_RECENT_SEARCHES) : []
  } catch { return [] }
}

export function pushRecentSearch(term: string, expectedContextKey: string | null): void {
  const key = storageKey(expectedContextKey)
  const trimmed = term.trim()
  if (!key || !trimmed) return
  const next = [trimmed, ...readRecentSearches(expectedContextKey).filter(value => value !== trimmed)].slice(0, MAX_RECENT_SEARCHES)
  try { localStorage.setItem(key, JSON.stringify(next)) } catch { /* Optional personal preference. */ }
}
