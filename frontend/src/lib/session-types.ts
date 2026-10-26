import type { ContextIdentity } from "@/lib/context-identity"

export interface SessionFence { readonly sessionId: string | null; readonly version: number; readonly contextKey: string | null }
export interface SessionRecord {
  version: number
  sessionId: string | null
  status: "active" | "anonymous" | "unknown" | "in-flight"
  token: string | null
  identity: ContextIdentity | null
  contextKey: string | null
}
export type SessionStatus = "initializing" | "active" | "anonymous" | "refreshing" | "switching" | "unknown" | "unavailable"
export interface SessionObservation { record: SessionRecord | null; status: SessionStatus; error: string | null }

export class SessionCoordinationError extends Error {
  constructor(message: string, readonly code: "SESSION_UNAVAILABLE" | "SESSION_UNKNOWN" | "SESSION_CHANGED") {
    super(message)
    this.name = code === "SESSION_CHANGED" ? "AbortError" : "SessionCoordinationError"
  }
}

