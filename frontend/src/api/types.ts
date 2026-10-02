/**
 * Dante Cloud API 强类型定义(与 docs/api-contract.md 精确一致)
 */

export type Status = "ENABLED" | "DISABLED"
export type MenuType = "DIR" | "MENU" | "BUTTON"
export type ClientType = "CONFIDENTIAL" | "PUBLIC"
export type DeviceStatus = "PENDING" | "APPROVED" | "DENIED" | "EXPIRED"
export type ApplicationRuntimeMode = "IFRAME" | "TRUSTED"
export type ApplicationEntryType = "ESM" | "HTML"
export type ApplicationEnvironment = "development" | "staging" | "production"

export interface PageResp<T> {
  records: T[]
  total: number
  page: number
  size: number
}

export interface PageParams {
  page?: number
  size?: number
  keyword?: string
}

/* ============ 1. 认证 ============ */

export interface Membership {
  organizationId: string
  organizationName: string
  organizationPath: string
  tenantId: string
  tenantName: string
  departmentId?: string | null
  departmentName?: string | null
}

export interface LoginResp {
  memberships: Membership[]
  ticket?: string
}

export interface AuthUser {
  id: string
  username: string
  realName: string
  email: string
  phone: string
  avatar: string
  gender: string
  isSuperAdmin: boolean
}

export interface TenantBrief {
  id: string
  name: string
  code: string
  /** 平台租户标记: 平台租户成员可跨租户管理 (后端会话上下文返回) */
  isPlatform?: boolean
}

export interface OrgBrief {
  id: string
  name: string
  path: string
}

export interface AppBrief {
  id: string
  code: string
  name: string
  icon: string
  basePath: string
  /** 默认 iframe 隔离；仅显式标记的自有可信应用可在宿主内运行。 */
  runtimeMode?: ApplicationRuntimeMode
  /** 当前宿主环境的活动版本入口类型。 */
  entryType?: ApplicationEntryType
  /** 当前宿主环境的活动版本入口地址；未发布时使用应用配置。 */
  entryUrl?: string
  deploymentVersion?: string | null
  entryVersionId?: string | null
}

export interface ContextResp {
  accessToken: string
  expiresIn: number
  user: AuthUser
  tenant: TenantBrief
  organization: OrgBrief
}

export interface ProfileResp {
  user: AuthUser
  tenant: TenantBrief
  organization: OrgBrief
  applications: AppBrief[]
  memberships: Membership[]
}

export interface MenuNode {
  id: string
  parentId: string | null
  type: MenuType
  title: string
  path: string | null
  icon: string | null
  permission: string | null
  sortOrder: number
  visible: boolean
  component?: string | null
  /** 外链地址: 空串/缺省 = 非外链; 非空时以 http(s):// 开头 */
  linkUrl?: string
  /** 外链打开方式: BLANK = 新窗口, SELF = 内部嵌入(/embed) */
  linkTarget?: string
  children?: MenuNode[]
}

export interface PermissionsResp {
  permissions: string[]
  roles: { id: string; name: string; code: string }[]
}

/* ============ 2.0-bis 个人安全中心 ============ */

/** 本人登录历史记录(安全中心时间线, 含成功与失败) */
export interface LoginHistoryRecord {
  id: number | string
  action: "LOGIN" | "LOGIN_FAILED"
  ip: string | null
  detail: string | null
  tenantName: string | null
  organizationName: string | null
  createdAt: string
}

export interface LoginHistoryPage {
  records: LoginHistoryRecord[]
  total: number
  page: number
  size: number
}

/** 本人名下按 OAuth 客户端聚合的会话概览 */
export interface MySessionRecord {
  clientId: string
  activeAccess: number
  activeRefresh: number
  lastIssuedAt: string | null
  lastExpiresAt: string | null
  scope: string
}

export interface MySessionsResp {
  sessions: MySessionRecord[]
  revokedTotal: number
  platformSessions: PlatformSessionRecord[]
}

export interface PlatformSessionRecord {
  id: string
  tenantId: string
  tenantName: string
  organizationId: string
  organizationName: string
  issuedAt: string
  expiresAt: string
  revokedAt: string | null
  ip: string
  userAgent: string
  active: boolean
  isCurrent: boolean
}

/* ============ 2.1 租户 ============ */

export interface TenantRecord {
  id: string
  name: string
  code: string
  description: string | null
  status: Status
  /** 平台租户标记: 全库唯一, 不可删除/停用 */
  isPlatform?: boolean
  /** 过期时间(ISO, null/缺省=永不过期; 到期后租户下用户无法登录) */
  expiresAt?: string | null
  createdAt: string
  organizationCount: number
  userCount: number
  authorizedAppCount: number
}

export interface TenantPayload {
  name: string
  code: string
  description?: string
  /** 过期时间(ISO; null=清除/永不过期) */
  expiresAt?: string | null
}

/* ============ 2.2 授权 ============ */

export interface AuthorizationApp {
  id: string
  code: string
  name: string
  icon: string
  authorized: boolean
  menuCount: number
  authorizedMenuCount: number
}

export interface AuthorizationMenu {
  id: string
  parentId: string | null
  type?: MenuType
  title?: string
  path?: string | null
  icon?: string | null
  sortOrder?: number
  authorized: boolean
}

export interface AuthorizationResp {
  applications: AuthorizationApp[]
  menus: Record<string, AuthorizationMenu[]>
}

export interface AuthorizationPayload {
  applications: { applicationId: string; authorized: boolean }[]
  menus: { menuId: string; authorized: boolean }[]
}

/* ============ 2.3 应用 ============ */

export interface ApplicationRecord {
  id: string
  code: string
  name: string
  icon: string
  basePath: string
  description: string | null
  sortOrder: number
  status: Status
  menuCount: number
  runtimeMode?: ApplicationRuntimeMode
  /** 应用基础入口类型，环境部署可通过版本指针选择活动入口。 */
  entryType?: ApplicationEntryType
  entryUrl?: string | null
}

export interface ApplicationPayload {
  code: string
  name: string
  icon: string
  basePath: string
  description?: string
  sortOrder: number
  status: Status
  runtimeMode?: ApplicationRuntimeMode
  /** iframe 应用仅支持 HTML；可信应用支持 HTML / ESM。 */
  entryType?: ApplicationEntryType
  /** HTML 入口留空时由后端推导为 /sub-apps/{code}/index.html */
  entryUrl?: string | null
  /** 新建 HTML 微应用时，一并创建 /micro/{code} 首页菜单 */
  autoCreateMenu?: boolean
}

export type ApplicationResourceKind = "SCRIPT" | "STYLE" | "ASSET"
export type ApplicationValidationStatus = "DRAFT" | "PASSED" | "FAILED"

export interface ApplicationReleaseResource {
  url: string
  kind: ApplicationResourceKind
}

export interface ApplicationReleaseCheck {
  url: string
  kind: string
  passed: boolean
  statusCode?: number | null
  contentType?: string | null
  bytes?: number | null
  message: string
}

export interface ApplicationReleaseValidation {
  /** 未预检时后端返回空对象，字段仅在完成预检后出现。 */
  passed?: boolean
  checkedAt?: string
  checks?: ApplicationReleaseCheck[]
}

export interface ApplicationReleaseRecord {
  id: string
  applicationId: string
  environment: ApplicationEnvironment
  version: string
  entryType: ApplicationEntryType
  entryUrl: string
  healthUrl: string | null
  resources: ApplicationReleaseResource[]
  validationStatus: ApplicationValidationStatus
  validationResult: ApplicationReleaseValidation | null
  checkedAt: string | null
  active: boolean
  createdAt: string
}

export interface ApplicationReleasePayload {
  environment: ApplicationEnvironment
  version: string
  entryType: ApplicationEntryType
  entryUrl: string
  healthUrl?: string
  resources?: ApplicationReleaseResource[]
}

export interface ApplicationDeploymentRecord {
  applicationId: string
  environment: ApplicationEnvironment
  activeReleaseId: string
  previousReleaseId: string | null
  version: string
  entryType: ApplicationEntryType
  entryUrl: string
  updatedAt: string
}

/* ============ 2.4 菜单 ============ */

export interface MenuPayload {
  applicationId: string
  parentId: string | null
  type: MenuType
  title: string
  path?: string
  icon?: string
  component?: string
  permission?: string
  sortOrder: number
  visible: boolean
  /** 外链地址(空串 = 清除外链) */
  linkUrl?: string
  /** 打开方式 BLANK/SELF(仅 linkUrl 非空时有意义) */
  linkTarget?: string
}

/* ============ 2.5 组织 ============ */

export interface OrgNode {
  id: string
  tenantId: string
  parentId: string | null
  name: string
  code: string
  sortOrder: number
  description: string | null
  status: Status
  /** 过期时间(ISO, null/缺省=永不过期; 到期后组织下用户无法登录) */
  expiresAt?: string | null
  /** 该组织直属部门数 */
  departmentCount: number
  /** 该组织直属成员数 */
  userCount: number
  children: OrgNode[]
}

export interface OrgPayload {
  tenantId: string
  parentId: string | null
  name: string
  code: string
  sortOrder: number
  description?: string
  /** 过期时间(ISO; null=清除/永不过期) */
  expiresAt?: string | null
}

/* ============ 2.7 部门 ============ */

export interface DeptNode {
  id: string
  organizationId: string
  parentId: string | null
  name: string
  code: string
  sortOrder: number
  description: string | null
  status?: Status
  children: DeptNode[]
}

export interface DeptPayload {
  organizationId: string
  parentId: string | null
  name: string
  code: string
  sortOrder: number
  description?: string
}

/* ============ 2.8 用户 ============ */

export interface UserRecord {
  id: string
  username: string
  realName: string
  email: string
  phone: string
  gender: string
  status: Status
  lastLoginAt: string | null
  createdAt: string
  memberships: {
    organizationId: string
    organizationName: string
    departmentId: string | null
    departmentName: string | null
  }[]
  roleCount: number
}

export interface UserPayload {
  username: string
  password: string
  realName: string
  email?: string
  phone?: string
  gender?: string
}

export interface UserMembershipItem {
  organizationId: string
  tenantId: string
  departmentId: string | null
  position: string
}

export interface UserRoleItem {
  roleId: string
  organizationId: string
}

/* ============ 2.9 角色 ============ */

export interface RoleRecord {
  id: string
  name: string
  code: string
  description: string | null
  organizationId: string
  organizationName: string
  tenantId: string
  status: Status
  builtIn: boolean
  menuCount: number
  userCount: number
  createdAt: string
}

export interface RolePayload {
  name: string
  code: string
  description?: string
  organizationId: string
}

/* ============ 2.10 OAuth ============ */

export interface OAuthClientRecord {
  id: string
  clientId: string
  clientName: string
  clientType: ClientType
  description?: string | null
  redirectUris: string[]
  grantTypes: string[]
  scopes: string[]
  accessTokenValidity: number
  refreshTokenValidity: number
  status: Status
  /** 关联平台应用(21-e 后端契约, 联调期间可能缺失: null/undefined = 未关联) */
  applicationId?: string | null
  applicationName?: string | null
  createdAt: string
}

export interface OAuthClientPayload {
  clientName: string
  clientId?: string
  clientType: ClientType
  redirectUris: string[]
  grantTypes: string[]
  scopes: string[]
  accessTokenValidity: number
  refreshTokenValidity: number
  description?: string
  /** 关联平台应用: null = 清除关联(21-e 后端契约) */
  applicationId?: string | null
}

export interface OAuthClientCreateResp {
  clientSecret?: string
  clientId?: string
  [k: string]: unknown
}

export interface OAuthTokenRecord {
  id: string
  tokenType: "access" | "refresh"
  clientId: string
  username: string
  scope: string
  createdAt: string
  expiresAt: string
  revoked: boolean
}

export interface DeviceCodeRecord {
  id: string
  userCode: string
  clientId: string
  clientName: string
  scope: string
  status: DeviceStatus
  username: string | null
  createdAt: string
  expiresAt: string
}

/** 微前端会话令牌响应(POST /api/oauth/session-token body {appCode} → 201; 失败 4xx) */
export interface OAuthSessionTokenResp {
  accessToken: string
  expiresIn: number
  tokenType: string
  audience: string
}

/** OAuth 会话视图:按用户聚合的令牌概览 (GET /api/oauth/tokens/sessions) */
export interface OAuthSession {
  userId: string
  username: string
  realName: string
  clients: string[]
  activeAccess: number
  activeRefresh: number
  lastIssuedAt: string | null
  lastExpiresAt: string | null
  revokedCount: number
}

/* ============ 2.11 审计 ============ */

export interface AuditLogRecord {
  id: string
  username: string
  /** 行所属租户 id(null=租户为空的平台级系统事件行) */
  tenantId?: string | null
  tenantName: string | null
  organizationName: string | null
  action: string
  detail: string | null
  ip: string | null
  /** 客户端 User-Agent(可能为空串; 旧后端未返回时缺省) */
  userAgent?: string
  createdAt: string
}

/* ============ 2.12 监控 ============ */

export interface MonitorOverview {
  tenantCount: number
  organizationCount: number
  userCount: number
  roleCount: number
  applicationCount: number
  menuCount: number
  oauthClientCount: number
  activeTokenCount: number
  todayLoginCount: number
  recentLogins: { username: string; organizationName: string | null; ip: string | null; createdAt: string }[]
  loginTrend7d: { date: string; count: number }[]
  auditTrend7d: { date: string; count: number }[]
}

export interface HealthResp {
  status: string
  version: string
  database: string
  service?: string
}

/** 平台监控系统信息 (GET /api/monitor/system, 权限 system:monitor, 后端契约 18-b)
 *  字段全部防御式可空: 旧后端 404 时整卡走错误态, 新后端缺字段时不崩溃 */
export interface MonitorSystemInfo {
  service: {
    name: string | null
    version: string | null
    pid: number | null
    startedAt: string | null
    uptimeSeconds: number | null
    /** 常驻内存 (MB) */
    memoryRssMb: number | null
  }
  database: {
    product: string | null
    serverStartedAt: string | null
    databaseSizeBytes: number | null
    connections: { total: number | null; active: number | null; idle: number | null } | null
    poolSize: number | null
    /** 核心表清单(按大小排序前在前端再排) */
    tables: { name: string; rows: number | null; sizeBytes: number | null }[] | null
  }
}

/* ============ 3. OAuth 对外 ============ */

export interface ConsentInfo {
  client: { clientId: string; clientName: string; description: string | null }
  scope: string
  redirectUri: string
  state: string | null
  expiresAt: string
}

export interface ConsentDecisionResp {
  redirectTo: string
}

export interface DeviceInfo {
  clientId: string
  clientName: string
  scope: string
  status: DeviceStatus
  expiresAt: string
}

export interface OAuthDiscovery {
  issuer: string
  authorization_endpoint: string
  token_endpoint: string
  device_authorization_endpoint: string
  userinfo_endpoint: string
  introspection_endpoint: string
  revocation_endpoint: string
  response_types_supported: string[]
  grant_types_supported: string[]
  subject_types_supported: string[]
  code_challenge_methods_supported: string[]
  [k: string]: unknown
}

/* ============ 通知中心 ============ */

export type NotificationType = "INFO" | "SUCCESS" | "WARNING" | "SECURITY"

export interface NotificationRecord {
  id: string
  userId: string
  tenantId: string
  organizationId: string
  sourceKind: "APP" | "SYSTEM"
  applicationId: string | null
  publicationId: string | null
  applicationName: string | null
  type: NotificationType
  title: string
  content: string
  link: string
  isRead: boolean
  readAt: string | null
  createdAt: string
}

export interface NotificationSendBody {
  applicationId: string
  idempotencyKey: string
  target: NotificationTarget
  title: string
  content: string
  type: NotificationType
  link?: string
}

export type NotificationTarget =
  | { kind: "USER"; userId: string; organizationId: string }
  | { kind: "ORGANIZATION"; organizationId: string }
  | { kind: "TENANT" }

export interface NotificationPublishResponse {
  publicationId: string
  recipientCount: number
  deliveryCount: number
  duplicate: boolean
  createdAt: string
}

export interface NotificationSendResponse extends NotificationPublishResponse { sent: number }

/** Public Socket.IO acknowledgment; sessionId is validated only inside the realtime service. */
export interface NotificationSocketContext {
  contractVersion: "1.0"
  userId: string
  tenantId: string
  organizationId: string
  expiresAt: string
}

export interface RealtimeNotification extends Pick<NotificationRecord,
  "id" | "userId" | "tenantId" | "organizationId" | "applicationId" | "publicationId" |
  "type" | "title" | "content" | "link" | "createdAt"> { contractVersion: "1.0" }
