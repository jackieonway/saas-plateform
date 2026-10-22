import type { ApplicationSetupStatus } from "@/api/application-setup"
import type { ApplicationEnvironment, ApplicationRecord } from "@/api/types"

export type ApplicationSetupStep = "overview" | "release" | "access"
export type SetupCheck = {
  id: "application" | "release" | "menus" | "access"
  title: string
  description: string
  complete: boolean
  action?: ApplicationSetupStep | "menus"
}

export function setupEnvironment(raw: string | null): ApplicationEnvironment {
  return raw === "production" || raw === "staging" ? raw : "development"
}

export function setupStep(raw: string | null): ApplicationSetupStep {
  return raw === "release" || raw === "access" ? raw : "overview"
}

/** Status describes only the selected target; an unspecified audience is never ready. */
export function applicationSetupChecks(
  application: ApplicationRecord,
  status: ApplicationSetupStatus,
  hostModuleAvailable: boolean,
): SetupCheck[] {
  const releaseReady = status.releaseRequired
    ? Boolean(status.activeVersion && status.releaseValidated)
    : status.releaseValidated && (status.entryType !== "ESM" || hostModuleAvailable)
  const access = status.access
  const accessConfigured = !!access && access.tenantAuthorized && access.organizationAuthorized && access.roleAuthorized
  return [
    { id: "application", title: "应用信息", complete: status.applicationEnabled,
      description: status.applicationEnabled ? `${application.name} · ${application.code}` : "应用当前已停用，请在应用信息中启用。" },
    { id: "release", title: status.releaseRequired ? "发布版本" : "宿主模块", complete: releaseReady, action: "release",
      description: status.releaseRequired
        ? releaseReady ? `当前已激活 ${status.activeVersion}` : status.activeVersion ? "当前版本尚未通过检查，请重新预检。" : "上传构建包或填写 HTML 入口，检查后确认发布。"
        : releaseReady ? "入口已配置，模块随宿主版本发布。" : "入口或宿主模块尚未就绪，请核对已构建模块。" },
    { id: "menus", title: "访问页面", complete: status.entryMenuCount > 0, action: "menus",
      description: status.entryMenuCount > 0 ? `已配置 ${status.entryMenuCount} 个可访问页面，共 ${status.menuCount} 个菜单项。` : "还没有可访问的菜单页面，请先配置首页。" },
    { id: "access", title: "访问范围", complete: accessConfigured && access.assignedUserCount > 0, action: "access",
      description: !access ? "选择租户、组织和角色后，检查实际访问范围。"
        : !accessConfigured ? "当前所选范围的应用或首页菜单授权尚未完整。"
          : access.assignedUserCount === 0 ? "功能授权已配置，该角色尚无当前组织内的有效成员。"
            : `所选范围的功能授权已配置，${access.assignedUserCount} 名有效成员关联此角色。` },
  ]
}
