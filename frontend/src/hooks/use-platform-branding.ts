import { useEffect } from "react"
import { useQuery } from "@tanstack/react-query"
import { platformApi, type PlatformBranding } from "@/api/platform"

export const PLATFORM_BRANDING_QUERY_KEY = ["platform-branding"] as const

export const DEFAULT_PLATFORM_BRANDING: PlatformBranding = {
  platformName: "管理平台",
  platformDescription: "多租户多应用统一权限平台",
  logoUrl: "",
  loginTitle: "欢迎回来",
  loginDescription: "登录平台，管理您的租户与应用权限",
  footerText: "",
}

function useBrandingQuery(refresh = false) {
  return useQuery({
    queryKey: PLATFORM_BRANDING_QUERY_KEY,
    queryFn: ({ signal }) => platformApi.branding(signal),
    staleTime: 30_000,
    retry: 1,
    refetchOnWindowFocus: true,
    refetchInterval: refresh ? 30_000 : false,
  })
}

/** 公开品牌配置与已登录页面共享缓存，加载失败时使用中性的默认文案。 */
export function usePlatformBranding(): PlatformBranding {
  return useBrandingQuery().data ?? DEFAULT_PLATFORM_BRANDING
}

/** 常驻应用根节点，持续同步后台配置，并更新浏览器描述与图标。 */
export function usePlatformBrandingMetadata() {
  const { data = DEFAULT_PLATFORM_BRANDING } = useBrandingQuery(true)
  useEffect(() => {
    let description = document.querySelector<HTMLMetaElement>('meta[name="description"]')
    if (!description) {
      description = document.createElement("meta")
      description.name = "description"
      document.head.appendChild(description)
    }
    description.content = data.platformDescription

    let icon = document.querySelector<HTMLLinkElement>('link[rel="icon"]')
    if (!icon) {
      icon = document.createElement("link")
      icon.rel = "icon"
      document.head.appendChild(icon)
    }
    icon.removeAttribute("type")
    icon.href = data.logoUrl || "/logo.svg"
  }, [data.platformDescription, data.logoUrl])
}
