/** Public micro-application contracts come from the registered SDK generator. */
export type {
  MicroEntryType,
  MicroRuntimeMode,
  MicroTheme,
  MicroNotifyVariant,
  MicroUser,
  MicroMenuItem,
  MicroTenant,
  MicroOrganization,
  MicroRole,
  MicroDataGrant,
  AppAuthorizationContext,
  MicroProps,
  MicroLifecycle,
  MicroEsmModule,
} from "../../../../examples/micro-app-sdk/generated/types"

import type { MicroEntryType, MicroRuntimeMode } from "../../../../examples/micro-app-sdk/generated/types"

/** 子应用全局生命周期挂载键。 */
export const MICRO_APP_GLOBAL = "__MICRO_APP__" as const

/** 容器侧从应用列表解析出的入口信息；不是 SDK 公开 DTO。 */
export interface ApplicationEntryInfo {
  id: string
  code: string
  name: string
  icon: string
  entryType: MicroEntryType
  runtimeMode: MicroRuntimeMode
  deploymentVersion?: string | null
  entryVersionId?: string | null
  entryUrl: string
}

/** 挂载状态机。 */
export type MicroStatus = "loading" | "mounted" | "error"

/** 令牌状态（状态条指示灯）。 */
export type MicroTokenState = "pending" | "injected" | "missing" | "unused"
