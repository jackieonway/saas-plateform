import { lazy } from "react"
import { Navigate, Route, Routes, useLocation } from "react-router-dom"
import { RouteRecoveryBoundary } from "@/components/RouteRecoveryBoundary"
import { useAuthStore } from "@/stores/auth"
import { usePlatformBrandingMetadata } from "@/hooks/use-platform-branding"

const AppLayout = lazy(() => import("@/components/layout/AppLayout"))
const LoginPage = lazy(() => import("@/pages/login/LoginPage"))
const DevicePage = lazy(() => import("@/pages/device/DevicePage"))
const ConsentPage = lazy(() => import("@/pages/consent/ConsentPage"))
const DashboardPage = lazy(() => import("@/pages/dashboard/DashboardPage"))
const ProfilePage = lazy(() => import("@/pages/profile/ProfilePage"))
const EmbedPage = lazy(() => import("@/pages/embed/EmbedPage"))
const TenantsPage = lazy(() => import("@/pages/system/tenants/TenantsPage"))
const PlatformSettingsPage = lazy(() => import("@/pages/system/settings/PlatformSettingsPage"))
const ApplicationsPage = lazy(() => import("@/pages/system/applications/ApplicationsPage"))
const MenusPage = lazy(() => import("@/pages/system/menus/MenusPage"))
const OrganizationsPage = lazy(() => import("@/pages/system/organizations/OrganizationsPage"))
const DepartmentsPage = lazy(() => import("@/pages/system/departments/DepartmentsPage"))
const AuthorizationsPage = lazy(() => import("@/pages/system/authorizations/AuthorizationsPage"))
const DataGrantsPage = lazy(() => import("@/pages/system/data-grants/DataGrantsPage"))
const UsersPage = lazy(() => import("@/pages/system/users/UsersPage"))
const CustomersPage = lazy(() => import("@/pages/system/customers/CustomersPage"))
const RolesPage = lazy(() => import("@/pages/system/roles/RolesPage"))
const RegionsPage = lazy(() => import("@/pages/system/regions/RegionsPage"))
const OAuthClientsPage = lazy(() => import("@/pages/system/oauth/ClientsPage"))
const OAuthTokensPage = lazy(() => import("@/pages/system/oauth/TokensPage"))
const AuditLogsPage = lazy(() => import("@/pages/system/audit/AuditLogsPage"))
const MonitorPage = lazy(() => import("@/pages/system/monitor/MonitorPage"))
const NotificationsPage = lazy(() => import("@/pages/system/notifications/NotificationsPage"))
const AthenaOverviewPage = lazy(() => import("@/pages/athena/OverviewPage"))
const AthenaSalesPage = lazy(() => import("@/pages/athena/ReportsSalesPage"))
const AthenaOperationsPage = lazy(() => import("@/pages/athena/ReportsOperationsPage"))
const AthenaSourcesPage = lazy(() => import("@/pages/athena/SourcesPage"))
const HermesInboxPage = lazy(() => import("@/pages/hermes/InboxPage"))
const HermesNotificationsPage = lazy(() => import("@/pages/hermes/NotificationsPage"))
const MicroAppContainer = lazy(() => import("@/components/micro/MicroAppContainer"))

function ProtectedLayout() {
  const token = useAuthStore((state) => state.token)
  const { pathname, search } = useLocation()
  if (!token) return <Navigate to={`/login?return=${encodeURIComponent(pathname + search)}`} replace />
  return <AppLayout />
}

function NotFound() {
  return (
    <div className="flex min-h-[60vh] flex-col items-center justify-center gap-3 text-center">
      <p className="font-mono text-5xl font-bold text-primary/30">404</p>
      <h1 className="text-lg font-semibold">页面不存在</h1>
      <p className="max-w-sm text-sm text-muted-foreground">
        您访问的页面可能已被移除,或您所在的组织未获授权访问该应用。
      </p>
    </div>
  )
}

export default function App() {
  usePlatformBrandingMetadata()
  const { pathname } = useLocation()
  return (
    <RouteRecoveryBoundary key={pathname === "/login" || pathname === "/device" || pathname === "/consent" ? pathname : "protected"}>
      <Routes>
        {/* 公开路由 */}
        <Route path="/login" element={<LoginPage />} />
        <Route path="/device" element={<DevicePage />} />
        <Route path="/consent" element={<ConsentPage />} />

        {/* 受保护路由(AppLayout 内含登录守卫) */}
        <Route element={<ProtectedLayout />}>
          <Route path="/" element={<Navigate to="/dashboard" replace />} />
          <Route path="/dashboard" element={<DashboardPage />} />
          <Route path="/profile" element={<ProfilePage />} />
          {/* 外链菜单内部嵌入页(SELF 打开方式) */}
          <Route path="/embed" element={<EmbedPage />} />

          <Route path="/system/tenants" element={<TenantsPage />} />
          <Route path="/system/settings" element={<PlatformSettingsPage />} />
          <Route path="/system/applications" element={<ApplicationsPage />} />
          <Route path="/system/menus" element={<MenusPage />} />
          <Route path="/system/organizations" element={<OrganizationsPage />} />
          <Route path="/system/departments" element={<DepartmentsPage />} />
          <Route path="/system/authorizations" element={<AuthorizationsPage />} />
          <Route path="/system/data-grants" element={<DataGrantsPage />} />
          <Route path="/system/users" element={<UsersPage />} />
          <Route path="/system/customers" element={<CustomersPage />} />
          <Route path="/system/roles" element={<RolesPage />} />
          <Route path="/system/regions" element={<RegionsPage />} />
          <Route path="/system/oauth/clients" element={<OAuthClientsPage />} />
          <Route path="/system/oauth/tokens" element={<OAuthTokensPage />} />
          <Route path="/system/audit-logs" element={<AuditLogsPage />} />
          <Route path="/system/monitor" element={<MonitorPage />} />
          <Route path="/system/notifications" element={<NotificationsPage />} />

          <Route path="/athena/overview" element={<AthenaOverviewPage />} />
          <Route path="/athena/reports/sales" element={<AthenaSalesPage />} />
          <Route path="/athena/reports/operations" element={<AthenaOperationsPage />} />
          <Route path="/athena/sources" element={<AthenaSourcesPage />} />

          <Route path="/hermes/inbox" element={<HermesInboxPage />} />
          <Route path="/hermes/notifications" element={<HermesNotificationsPage />} />

          {/* /micro/{appCode}/* 由 qiankun 加载独立 HTML 应用；内置页面沿用上方 React 路由。 */}
          <Route path="/micro/:appCode/*" element={<MicroAppContainer />} />

          <Route path="*" element={<NotFound />} />
        </Route>
      </Routes>
    </RouteRecoveryBoundary>
  )
}
