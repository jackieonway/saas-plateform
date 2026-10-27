/** 路由标题映射(用于面包屑兜底与命令面板) */
export const ROUTE_TITLES: Record<string, string> = {
  "/dashboard": "仪表盘",
  "/profile": "个人中心",
  "/system/tenants": "租户管理",
  "/system/settings": "平台设置",
  "/system/applications": "应用管理",
  "/system/menus": "菜单管理",
  "/system/organizations": "组织管理",
  "/system/departments": "部门管理",
  "/system/authorizations": "授权管理",
  "/system/users": "用户管理",
  "/system/customers": "客户管理",
  "/system/roles": "角色管理",
  "/system/regions": "地区管理",
  "/system/oauth/clients": "OAuth 客户端",
  "/system/oauth/tokens": "设备与会话",
  "/system/audit-logs": "审计日志",
  "/system/monitor": "平台监控",
  "/system/notifications": "通知中心",
  "/athena/overview": "数据总览",
  "/athena/reports/sales": "销售报表",
  "/athena/reports/operations": "运营报表",
  "/athena/sources": "数据源",
  "/hermes/inbox": "收件箱",
  "/hermes/notifications": "通知设置",
}

export function routeTitle(pathname: string): string {
  if (ROUTE_TITLES[pathname]) return ROUTE_TITLES[pathname]
  const seg = pathname.split("/").filter(Boolean).pop() ?? ""
  return seg ? decodeURIComponent(seg) : pathname
}
