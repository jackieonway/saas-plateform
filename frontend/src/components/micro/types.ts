/** Public micro-application contracts come from the registered SDK generator. */
export type {
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
} from "../../../vendor/micro-app-sdk/types"

import type { MicroProps } from "../../../vendor/micro-app-sdk/types"

/** Host-owned handle for sequencing qiankun work and authorization cleanup. */
export interface MicroApplicationHandle {
  name: string
  mount: (container: HTMLElement, props: MicroProps) => void | Promise<void>
  update?: (props: MicroProps) => void | Promise<void>
  unmount: () => void | Promise<void>
}

/** 容器侧从应用列表解析出的入口信息；不是 SDK 公开 DTO。 */
export interface ApplicationEntryInfo {
  id: string
  code: string
  name: string
  icon: string
  entryType: "HTML"
  runtimeMode: "TRUSTED"
  deploymentVersion?: string | null
  entryVersionId?: string | null
  entryUrl: string
}

/** 挂载状态机。 */
export type MicroStatus = "loading" | "mounted" | "error"

/** 令牌状态（状态条指示灯）。 */
export type MicroTokenState = "pending" | "injected" | "missing" | "unused"
