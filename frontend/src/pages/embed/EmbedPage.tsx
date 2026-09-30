import { useSearchParams } from "react-router-dom"
import { ExternalLink, ShieldAlert } from "lucide-react"
import { Button } from "@/components/ui/button"
import { PageHeader } from "@/components/layout/PageHeader"
import { EmptyState } from "@/components/states"
import { useDocumentTitle } from "@/hooks/use-document-title"

/** 仅放行 http/https 外链(防 javascript:/data: 等协议注入) */
function isSafeExternalUrl(raw: string): boolean {
  try {
    const parsed = new URL(raw)
    return parsed.protocol === "http:" || parsed.protocol === "https:"
  } catch {
    return false
  }
}

/**
 * 外链内部嵌入页(/embed?url=...&title=...):
 * 供 linkTarget=SELF 的外链菜单在应用内 iframe 打开, 剩余高度全部交给 iframe。
 */
export default function EmbedPage() {
  const [params] = useSearchParams()
  const url = (params.get("url") ?? "").trim()
  const title = (params.get("title") ?? "").trim() || "外部页面"

  // 标题随菜单标题动态变化(AppLayout 对 /embed 跳过标题覆盖)
  useDocumentTitle(title)

  if (!url) {
    return (
      <EmptyState
        className="py-20"
        title="缺少外链地址"
        description="请通过菜单的外链入口进入本页, 直接访问需携带 url 参数"
        icon={<ShieldAlert className="size-6" />}
      />
    )
  }

  if (!isSafeExternalUrl(url)) {
    return (
      <EmptyState
        className="py-20"
        title="不支持的外链地址"
        description="出于安全考虑, 仅允许 http:// 或 https:// 协议的外部页面"
        icon={<ShieldAlert className="size-6" />}
      />
    )
  }

  return (
    <div>
      <PageHeader
        title={title}
        description={
          <span className="font-mono text-xs" title={url}>
            {url}
          </span>
        }
        icon={<ExternalLink className="size-5" />}
        badge={
          <span className="rounded border bg-muted px-1.5 py-px text-[10px] font-medium leading-4 text-muted-foreground">
            外部页面
          </span>
        }
        actions={
          <Button variant="outline" onClick={() => window.open(url, "_blank", "noopener,noreferrer")}>
            <ExternalLink className="size-4" />
            在新窗口打开
          </Button>
        }
      />
      <iframe
        src={url}
        className="h-[calc(100dvh-13rem)] w-full rounded-xl border bg-card"
        sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
        referrerPolicy="no-referrer"
        title={title}
      />
    </div>
  )
}
