import type { ApplicationSetupStatus } from "@/api/application-setup"
import type { ApplicationRecord } from "@/api/types"

export type ApplicationSetupStep = "overview" | "release"
export const isQiankunApp = (app: Pick<ApplicationRecord, "entryType" | "runtimeMode">) =>
  app.entryType === "HTML" && app.runtimeMode === "TRUSTED"
export type SetupCheck = {
  id: "application" | "release" | "menus"
  title: string
  description: string
  complete: boolean
  action?: ApplicationSetupStep | "menus"
}

export function setupStep(raw: string | null): ApplicationSetupStep {
  return raw === "release" ? raw : "overview"
}

/** Onboarding checks deployment readiness; access is managed by the authorization pages. */
export function applicationSetupChecks(
  application: ApplicationRecord,
  status: ApplicationSetupStatus,
): SetupCheck[] {
  const directEntry = status.entrySource === "URL" || status.urlEntryConfigured === true
  const compatible = isQiankunApp(status)
  const releaseReady = compatible && (directEntry ? status.releaseValidated : status.releaseRequired
    ? Boolean(status.activeVersion && status.releaseValidated)
    : status.releaseValidated)
  return [
    { id: "application", title: "应用信息", complete: status.applicationEnabled,
      description: status.applicationEnabled ? `${application.name} · ${application.code}` : "应用当前已停用，请在应用信息中启用。" },
    { id: "release", title: directEntry && compatible ? "直接地址" : "应用入口", complete: releaseReady, action: "release",
      description: !compatible ? "此应用需重新配置 qiankun 入口；完成配置后才能运行。" : directEntry ? releaseReady ? `当前环境使用直接地址${status.proxyEnabled ? "，页面与资源通过平台代理。" : "。"}` : "直接地址尚未就绪，请核对入口与来源配置。" : status.releaseRequired
        ? releaseReady ? `当前已激活 ${status.activeVersion}` : status.activeVersion ? "当前版本尚未通过检查，请重新预检。" : "填写已部署应用的地址即可接入。"
        : releaseReady ? "qiankun 入口已配置。" : "qiankun 入口尚未就绪，请核对入口配置。" },
    { id: "menus", title: "访问页面", complete: status.entryMenuCount > 0, action: "menus",
      description: status.entryMenuCount > 0 ? `已配置 ${status.entryMenuCount} 个可访问页面，共 ${status.menuCount} 个菜单项。` : "还没有可访问的菜单页面，请先配置首页。" },

  ]
}
