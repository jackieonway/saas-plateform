/**
 * Dante Cloud 微前端契约类型(仿乾坤方案: HTML 入口内核 + 自研沙箱/编排)
 *
 * 与后端(21-a)/子应用服务(21-b)的共享契约,字段与行为必须精确一致:
 * - DB: applications.entry_type('ESM'|'HTML') + entry_url(HTML 时如 /sub/vue/)
 * - 子应用暴露 window.__DANTE_MICRO_APP__ = { name, mount(container, props), unmount() }
 */

/** 应用入口类型: ESM = 内置 ES 模块直挂; HTML = 微前端页面(沙箱执行) */
export type MicroEntryType = "ESM" | "HTML"

/** 只有平台管理员明确标记的自有代码可以在宿主页执行。 */
export type MicroRuntimeMode = "IFRAME" | "TRUSTED"

/** 主题通道: 主应用解析后的实际主题(system 跟随时已折叠为 light/dark) */
export type MicroTheme = "light" | "dark"

/** 子应用通知变体(映射主应用 toast) */
export type MicroNotifyVariant = "default" | "success" | "error" | "warning" | "info"

/** 注入子应用的当前用户信息 */
export interface MicroUser {
  id: string
  username: string
  /** 展示名(realName 优先, 缺省回退 username) */
  displayName: string
}

/** 注入子应用的菜单节点(主应用菜单树的结构子集, MenuNode 结构兼容可直接透传) */
export interface MicroMenuItem {
  id: string
  parentId: string | null
  type: "DIR" | "MENU" | "BUTTON"
  title: string
  path: string | null
  icon: string | null
  sortOrder: number
  visible: boolean
  permission?: string
  children?: MicroMenuItem[]
}

export interface MicroTenant { id: string; code: string; name: string }
export interface MicroOrganization { id: string; name: string; path: string }
export interface MicroRole { id: string; name: string; code: string }
export interface MicroDataGrant {
  resource: string
  action: string
  scope: "SELF" | "ORG" | "ORG_SUBTREE" | "TENANT"
  subjectUserId: string
  tenantId: string
  organizationIds: string[]
}

/** 平台 GET /api/apps/{appCode}/context 的用户视图。 */
export interface AppAuthorizationContext {
  contractVersion: "1.0"
  contextVersion: string
  expiresAt: string
  appCode: string
  application: {
    id: string
    name: string
    runtimeMode?: MicroRuntimeMode
    entryType?: MicroEntryType
    entryUrl?: string
    deploymentVersion?: string | null
    entryVersionId?: string | null
  }
  user: MicroUser
  tenant: MicroTenant
  organization: MicroOrganization
  /** 当前组织及其下级组织的硬边界；仅平台根上下文可明确返回 null。 */
  organizationScopeIds?: string[] | null
  roles: MicroRole[]
  permissions: string[]
  dataGrants: MicroDataGrant[]
  menus: MicroMenuItem[]
}

/** 注入子应用的 Props(共享契约,字段名与 21-b 子应用实现精确一致) */
export interface MicroProps {
  contractVersion: "2.0"
  contextVersion: string
  /** 宿主单调递增推送序号；用它拒绝旧请求结果。 */
  contextEpoch: number
  expiresAt: string
  tokenExpiresAt: string
  /** 不变发布版本；兼容期未注册发布时为空。 */
  deploymentVersion?: string | null
  entryVersionId?: string | null
  /** 上下文替换、过期、组织切换或卸载时中止；所有业务 fetch 都应绑定。 */
  signal: AbortSignal
  /** 应用编码(如 vue-workshop) */
  appCode: string
  /** 主应用内挂载基路径, 恒为 /micro/{appCode} */
  basePath: string
  /** 当前完整路由路径(如 /micro/vue-workshop/security), 子应用自行减去 basePath 得子路径 */
  routePath: string
  /** 短期应用令牌；未取得有效令牌和上下文时宿主不会挂载。 */
  token: string
  /** 当前用户 */
  user: MicroUser
  tenant: MicroTenant
  organization: MicroOrganization
  /** 所有数据授权都受此组织上限约束；缺少旧上下文字段时只开放当前组织。 */
  organizationScopeIds: string[] | null
  roles: MicroRole[]
  permissions: string[]
  dataGrants: MicroDataGrant[]
  /** 主应用当前主题 */
  theme: MicroTheme
  /** 该应用的菜单树(用户可见菜单; 无授权/拉取失败为空数组) */
  menus: MicroMenuItem[]
  /** 子应用请求主应用内部路由跳转(相对路径即可) */
  onNavigate: (path: string) => void
  /** 子应用请求主应用 toast 通知 */
  onNotify: (message: string, variant?: MicroNotifyVariant) => void
}

/** 子应用生命周期契约(由子应用在 window.__DANTE_MICRO_APP__ 上暴露) */
export interface MicroLifecycle {
  /** 子应用名(展示用) */
  name: string
  /** 挂载: 渲染到 container, props 见 MicroProps; 可为异步 */
  mount: (container: HTMLElement, props: MicroProps) => void | Promise<void>
  /** 卸载: 清理自身 DOM/监听/实例; 可为异步 */
  unmount: () => void | Promise<void>
  /**
   * 可选增量更新: 子路径/主题/菜单/令牌变化时由容器调用(不重挂)。
   * 未实现该方法的子应用, 路由变化时容器退回全量重挂(v1 行为, 向后兼容)。
   * 实现方应保持自身内部状态(计时器/输入/滚动位置)以证明「未重挂」。
   */
  update?: (props: MicroProps) => void | Promise<void>
}

/** ESM 入口模块形状(entry_type=ESM 时动态 import 的模块契约) */
export interface MicroEsmModule {
  __DANTE_MICRO_APP__?: MicroLifecycle
  mount?: MicroLifecycle["mount"]
  unmount?: MicroLifecycle["unmount"]
  update?: MicroLifecycle["update"]
}

/** 子应用全局生命周期挂载键 */
export const MICRO_APP_GLOBAL = "__DANTE_MICRO_APP__" as const

/** 容器侧从应用列表解析出的入口信息 */
export interface ApplicationEntryInfo {
  /** 应用 id(用于拉取该应用菜单树) */
  id: string
  code: string
  name: string
  icon: string
  entryType: MicroEntryType
  runtimeMode: MicroRuntimeMode
  deploymentVersion?: string | null
  entryVersionId?: string | null
  /** HTML 入口 URL(如 /sub/vue/); ESM 入口为模块 URL(本轮无实际使用方) */
  entryUrl: string
}

/** 挂载状态机 */
export type MicroStatus = "loading" | "mounted" | "error"

/** 令牌状态(状态条指示灯) */
export type MicroTokenState = "pending" | "injected" | "missing" | "unused"
