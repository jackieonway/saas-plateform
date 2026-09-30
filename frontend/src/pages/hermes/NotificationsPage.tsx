import * as React from "react"
import { Bell, Loader2, Save } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { Slider } from "@/components/ui/slider"
import { Separator } from "@/components/ui/separator"
import { PageHeader } from "@/components/layout/PageHeader"
import { DemoBadge } from "@/pages/athena/mock"
import { toast } from "sonner"

interface NotifySettings {
  securityEmail: boolean
  securitySms: boolean
  digest: boolean
  auditDaily: boolean
  mentions: boolean
  quietHours: boolean
  threshold: number
}

export default function NotificationsPage() {
  const [settings, setSettings] = React.useState<NotifySettings>({
    securityEmail: true,
    securitySms: false,
    digest: true,
    auditDaily: true,
    mentions: true,
    quietHours: false,
    threshold: 60,
  })
  const [saving, setSaving] = React.useState(false)

  const update = (patch: Partial<NotifySettings>) => setSettings((s) => ({ ...s, ...patch }))

  const save = async () => {
    setSaving(true)
    await new Promise((r) => setTimeout(r, 600))
    setSaving(false)
    toast.success("演示环境:已保存(本地)")
  }

  const rows: { key: keyof NotifySettings; title: string; desc: string }[] = [
    { key: "securityEmail", title: "安全事件 · 邮件通知", desc: "登录异常、令牌吊销、设备授权等安全事件实时邮件提醒" },
    { key: "securitySms", title: "安全事件 · 短信通知", desc: "高风险安全事件同时发送短信(适合管理员账号)" },
    { key: "digest", title: "每周摘要", desc: "每周一上午发送上周平台动态与数据摘要" },
    { key: "auditDaily", title: "审计日报", desc: "每日 9:00 推送前一日关键审计事件汇总" },
    { key: "mentions", title: "提及与回复", desc: "他人提及您或回复您的消息时立即通知" },
    { key: "quietHours", title: "免打扰时段(22:00 - 08:00)", desc: "免打扰期间消息进入收件箱,不推送提醒" },
  ]

  return (
    <div className="max-w-3xl space-y-6">
      <PageHeader
        title="通知设置"
        description="Hermes 消息中心 · 通知渠道与提醒偏好(演示环境,保存于本地)"
        icon={<Bell className="size-5" />}
        badge={<DemoBadge />}
      />

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">通知渠道</CardTitle>
          <CardDescription>选择希望接收提醒的事件与渠道</CardDescription>
        </CardHeader>
        <CardContent className="space-y-1">
          {rows.map((r, i) => (
            <div key={r.key}>
              {i > 0 && <Separator className="my-1" />}
              <div className="flex items-center justify-between gap-4 rounded-lg py-3.5">
                <div className="min-w-0">
                  <Label className="text-sm font-medium">{r.title}</Label>
                  <p className="mt-0.5 text-xs text-muted-foreground">{r.desc}</p>
                </div>
                <Switch
                  checked={settings[r.key] as boolean}
                  onCheckedChange={(v) => update({ [r.key]: v } as Partial<NotifySettings>)}
                  aria-label={r.title}
                />
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">告警阈值</CardTitle>
          <CardDescription>登录失败次数超过阈值时触发告警通知</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6 py-4">
          <div className="flex items-center justify-between">
            <Label className="text-sm">阈值(次 / 10 分钟)</Label>
            <span className="w-12 rounded-md bg-muted px-2 py-1 text-center text-sm font-semibold tabular-nums">
              {settings.threshold}
            </span>
          </div>
          <Slider
            value={[settings.threshold]}
            min={10}
            max={200}
            step={10}
            onValueChange={([v]) => update({ threshold: v })}
            aria-label="告警阈值"
          />
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>10 次(敏感)</span>
            <span>200 次(宽松)</span>
          </div>
        </CardContent>
      </Card>

      <div className="flex justify-end">
        <Button onClick={save} disabled={saving}>
          {saving ? <Loader2 className="size-4 animate-spin" /> : <Save className="size-4" />}
          保存设置
        </Button>
      </div>
    </div>
  )
}
