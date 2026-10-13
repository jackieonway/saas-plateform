import { useState } from "react"
import { usePlatformBranding } from "@/hooks/use-platform-branding"

/** 平台 Logo；未配置或图片加载失败时使用内置图形。 */
export function BrandMark({ className, logoUrl: overrideUrl }: { className?: string; logoUrl?: string }) {
  const { logoUrl: configuredUrl } = usePlatformBranding()
  const logoUrl = overrideUrl ?? configuredUrl
  const [failedUrl, setFailedUrl] = useState<string | null>(null)
  if (logoUrl && failedUrl !== logoUrl) {
    return (
      <span className={className} aria-hidden>
        <img src={logoUrl} alt="" className="size-full object-contain" onError={() => setFailedUrl(logoUrl)} />
      </span>
    )
  }
  return (
    <span className={className} aria-hidden>
      <svg viewBox="0 0 48 48" className="size-full">
        <defs>
          <linearGradient id="brand-g" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="currentColor" stopOpacity="0.95" />
            <stop offset="1" stopColor="currentColor" stopOpacity="0.55" />
          </linearGradient>
        </defs>
        <path
          d="M12 30a7 7 0 0 1 .5-13.96A9.5 9.5 0 0 1 30.6 14a8 8 0 0 1 7.9 9.4A6 6 0 0 1 37 31H12Z"
          fill="url(#brand-g)"
          opacity="0.9"
        />
        <path
          d="M24 26l4.2 4.2-1.7 1.7-1.5-1.5v6.1h-2v-6.1l-1.5 1.5-1.7-1.7L24 26Z"
          fill="currentColor"
        />
        <path
          d="M18.2 33.8a6.4 6.4 0 0 1-5.7-3.5"
          stroke="currentColor"
          strokeWidth="2.2"
          strokeLinecap="round"
          fill="none"
        />
      </svg>
    </span>
  )
}

export const BrandLogo = BrandMark

export function BrandWordmark({ className, light }: { className?: string; light?: boolean }) {
  const { platformName, platformDescription } = usePlatformBranding()
  return (
    <div className={`flex items-center gap-2.5 ${className ?? ""}`}>
      <span
        className={`flex size-9 items-center justify-center rounded-xl ${
          light ? "bg-white/10 text-white" : "bg-primary/10 text-primary"
        }`}
      >
        <BrandMark className="size-6" />
      </span>
      <div className="min-w-0 leading-tight">
        <p className={`truncate text-base font-semibold tracking-tight ${light ? "text-white" : ""}`} title={platformName}>{platformName}</p>
        <p className={`truncate text-[11px] ${light ? "text-white/60" : "text-muted-foreground"}`} title={platformDescription}>{platformDescription}</p>
      </div>
    </div>
  )
}
