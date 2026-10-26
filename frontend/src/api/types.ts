/**
 * SaaS Platform API 强类型定义；接口入口与边界见 backend/docs/architecture.md#api-catalog。
 */

import type { OrganizationType, TenantType } from "@/lib/entity-types"
export type { OrganizationType, TenantType } from "@/lib/entity-types"
import type { NotificationType, NotificationTarget,
  NotificationPublication as NotificationPublishResponse } from "../../vendor/micro-app-sdk/types"
export type { NotificationType, NotificationTarget, NotificationRecord, NotificationSocketContext,
  RealtimeNotification, NotificationPublication as NotificationPublishResponse } from "../../vendor/micro-app-sdk/types"

export type Status = "ENABLED" | "DISABLED"
export type MenuType = "DIR" | "MENU" | "BUTTON"
export type ClientType = "CONFIDENTIAL" | "PUBLIC"
export type DeviceStatus = "PENDING" | "APPROVED" | "DENIED" | "EXPIRED"
export type ApplicationRuntimeMode = "TRUSTED"
export type ApplicationEntryType = "HTML"
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

export type MemberType = "EMPLOYEE" | "CUSTOMER"

/** 服务端当前成员证明；版本使用规范十进制字符串，避免 i64 精度丢失。 */
export interface ContextMembershipProof {
  membershipId: string
  memberType: MemberType
  identityVersion: string
  accountVersion: string
}

export interface Membership {
  /** 登录选择项可关联多个已验证账号；建立会话时需提交对应 userId。 */
  userId: string
  username?: string
  realName?: string
  organizationId: string
  organizationName: string
  organizationPath: string
  tenantId: string
  tenantName: string
  departmentId?: string | null
  departmentName?: string | null
  membershipId: string
  memberType: MemberType
  identityVersion: string
  status: Status
  consoleSelectable: boolean
  selectionDisabledReason: null | "CLIENT_MEMBER_TYPE_NOT_ALLOWED" | "MEMBERSHIP_DISABLED"
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
  /** 平台租户属性；平台管理资格由会话的 platformContext 决定。 */
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
  /** 活动应用统一为可信 HTML；内置页面由 React 路由渲染。 */
  runtimeMode?: ApplicationRuntimeMode
  /** 当前宿主环境的活动版本入口类型。 */
  entryType?: ApplicationEntryType
  /** 微应用的有效 HTML 入口；内置页面不使用外部入口。 */
  entryUrl?: string | null
  deploymentVersion?: string | null
  entryVersionId?: string | null
}

export interface ContextResp extends ContextMembershipProof {
  accessToken: string
  expiresIn: number
  user: AuthUser
  tenant: TenantBrief
  organization: OrgBrief
  /** 当前会话位于平台租户根组织，具备平台管理上下文。 */
  platformContext?: boolean
}

export interface ProfileResp extends ContextMembershipProof {
  user: AuthUser
  tenant: TenantBrief
  organization: OrgBrief
  applications: AppBrief[]
  memberships: Membership[]
  /** 平台租户子组织为 false，不能据 tenant.isPlatform 扩大管理范围。 */
  platformContext?: boolean
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
  tenantType?: TenantType
  contactName?: string
  contactPhone?: string
  contactEmail?: string
  description: string | null
  province: string
  city: string
  district: string
  township: string
  village: string
  address: string
  longitude: number | null
  latitude: number | null
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
  tenantType?: TenantType
  contactName?: string
  contactPhone?: string
  contactEmail?: string
  description?: string
  province?: string
  city?: string
  district?: string
  township?: string
  village?: string
  address?: string
  longitude?: number | null
  latitude?: number | null
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

/** Only application administration reads archived, disabled entry metadata. */
export interface ArchivedApplicationEntryMetadata {
  /** Read-only persisted value; reconfiguration must use the active HTML contract. */
  entryType?: string
  runtimeMode?: string
}

export interface ApplicationRecord extends ArchivedApplicationEntryMetadata {
  id: string
  code: string
  name: string
  icon: string
  basePath: string
  description: string | null
  sortOrder: number
  status: Status
  menuCount: number
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
  /** 应用配置统一使用可信 HTML。 */
  entryType?: ApplicationEntryType
  /** 微应用需要 HTML 入口；内置页面保留空入口。 */
  entryUrl?: string | null
  /** 新建 HTML 微应用时，一并创建 /micro/{code} 首页菜单 */
  autoCreateMenu?: boolean
}

/** Environment-specific live entry. Its browser URL may use the platform proxy. */
export interface ApplicationUrlEntryRecord {
  applicationId: string
  environment: ApplicationEnvironment
  entryUrl: string
  proxyEnabled: boolean
  browserEntryUrl: string
  revision?: string
  updatedAt: string
}

export interface ApplicationUrlEntryPayload {
  entryUrl: string
  proxyEnabled: boolean
}

export type ApplicationResourceKind = "SCRIPT" | "STYLE" | "ASSET"
export type ApplicationValidationStatus = "DRAFT" | "PASSED" | "FAILED"

export interface ApplicationReleaseResource {
  url: string
  kind: ApplicationResourceKind
}

/** Register an already deployed HTML entry; deployment metadata stays with CI. */
export interface ApplicationReleasePayload {
  environment: ApplicationEnvironment
  version: string
  entryType: "HTML"
  entryUrl: string
  healthUrl: string | null
  resources: ApplicationReleaseResource[]
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
  /** Archived releases retain their persisted type for compatibility checks. */
  entryType: string
  entryUrl: string
  healthUrl: string | null
  resources: ApplicationReleaseResource[]
  validationStatus: ApplicationValidationStatus
  validationResult: ApplicationReleaseValidation | null
  checkedAt: string | null
  active: boolean
  createdAt: string
}

export interface ApplicationDeploymentRecord {
  applicationId: string
  environment: ApplicationEnvironment
  activeReleaseId: string
  previousReleaseId: string | null
  version: string
  /** A disabled application's deployment history may contain incompatible metadata. */
  entryType: string
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
  organizationType?: OrganizationType
  contactName?: string
  contactPhone?: string
  contactEmail?: string
  sortOrder: number
  description: string | null
  province: string
  city: string
  district: string
  township: string
  village: string
  address: string
  longitude: number | null
  latitude: number | null
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
  organizationType?: OrganizationType
  contactName?: string
  contactPhone?: string
  contactEmail?: string
  sortOrder: number
  description?: string
  province?: string
  city?: string
  district?: string
  township?: string
  village?: string
  address?: string
  longitude?: number | null
  latitude?: number | null
  /** 过期时间(ISO; null=清除/永不过期) */
  expiresAt?: string | null
}

/* ============ 2.7 部门 ============ */

export interface ContactAdministrator {
  alreadyInitialized?: boolean
  userId: string
  username: string
  roleName: string
  created: boolean
  initialPassword?: string
}

export interface EntityCreationResult {
  id: string
  administrator?: ContactAdministrator
}

/** 部门管理可切换的组织：当前登录组织及其下级组织。 */
export interface DepartmentOrgNode {
  id: string
  name: string
  code: string
  status?: Status
  children: DepartmentOrgNode[]
}

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
  /** Authorization-code requests must use PKCE with S256 when enabled. */
  requirePkce: boolean
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
  /** Omit on updates to retain the current policy; new PUBLIC clients default to true. */
  requirePkce?: boolean
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
export interface AdminPlatformSession extends PlatformSessionRecord {
  userId: string
  username: string
  realName: string
}

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
  requestId?: string | null
  applicationId?: string | null
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

export interface NotificationSendBody {
  applicationId: string
  idempotencyKey: string
  target: NotificationTarget
  title: string
  content: string
  type: NotificationType
  link?: string
}

export interface NotificationSendResponse extends NotificationPublishResponse { sent: number }
