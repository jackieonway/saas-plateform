import * as React from "react"
import { motion } from "framer-motion"
import { Archive, CheckCheck, Inbox as InboxIcon, Mail, MailOpen, MessageSquare, Trash2, User } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { Badge } from "@/components/ui/badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Separator } from "@/components/ui/separator"
import { PageHeader } from "@/components/layout/PageHeader"
import { EmptyState } from "@/components/states"
import { DemoBadge } from "@/pages/athena/mock"
import { formatDateTime, formatRelative } from "@/lib/format"
import { cn } from "@/lib/utils"

interface MailItem {
  id: string
  from: string
  fromEmail: string
  title: string
  preview: string
  body: string[]
  date: string
  read: boolean
  starred: boolean
  tag: "通知" | "公告" | "工单" | "系统"
}

const SEED: MailItem[] = [
  {
    id: "m1",
    from: "平台安全中心",
    fromEmail: "security@dante.cloud",
    title: "【安全提醒】检测到新的设备授权请求",
    preview: "智能电视客户端(smart-tv)于今日发起设备码授权,若非本人操作请及时吊销令牌...",
    body: [
      "您好,",
      "我们检测到智能电视客户端 smart-tv 于今日 14:22 通过设备码流程发起了授权请求,并在您批准后获得了访问令牌(scope: profile read)。",
      "若该操作并非您本人发起,请前往「设备与会话」页面立即吊销相关令牌,并建议尽快修改账户密码。",
      "—— Dante Cloud 平台安全中心",
    ],
    date: new Date(Date.now() - 18 * 60_000).toISOString(),
    read: false,
    starred: true,
    tag: "系统",
  },
  {
    id: "m2",
    from: "研发部",
    fromEmail: "rd@dante.cloud",
    title: "Athena 数据分析 v2.4 发布说明",
    preview: "本次更新包含漏斗分析、渠道归因与导出模板三大特性,以及 11 项问题修复...",
    body: [
      "各位同事:",
      "Athena 数据分析 v2.4 已正式发布,主要内容如下:",
      "1. 新增转化漏斗分析,支持自定义环节与时间窗口;",
      "2. 渠道归因模型升级为数据驱动归因(DDA);",
      "3. 报表导出支持自定义模板与定时推送;",
      "4. 修复 11 项已知问题,图表加载性能提升约 40%。",
      "如遇问题请在工单系统反馈。",
    ],
    date: new Date(Date.now() - 3 * 3600_000).toISOString(),
    read: false,
    starred: false,
    tag: "公告",
  },
  {
    id: "m3",
    from: "人事行政部",
    fromEmail: "hr@dante.cloud",
    title: "关于 2026 年 Q1 全员大会安排的通知",
    preview: "全员大会定于 3 月 18 日(周三)14:00 在总部 A101 会议室举行,同步线上直播...",
    body: [
      "全体同事:",
      "2026 年 Q1 全员大会定于 3 月 18 日(周三)14:00 举行,地点:总部 A101 会议室,线上直播同步开放。",
      "议程包括:季度经营回顾、组织架构调整说明、优秀团队表彰等。",
      "请合理安排工作,准时参加。",
    ],
    date: new Date(Date.now() - 26 * 3600_000).toISOString(),
    read: true,
    starred: false,
    tag: "通知",
  },
  {
    id: "m4",
    from: "IT 服务台",
    fromEmail: "itsupport@dante.cloud",
    title: "工单 #20481 已处理完成:VPN 无法连接",
    preview: "您提交的工单「VPN 无法连接」已处理完成,请重新拨入并确认访问正常...",
    body: [
      "您好:",
      "您提交的工单 #20481(VPN 无法连接)已处理完成。",
      "处理说明:已为您重置 VPN 证书并更新客户端配置,请重新拨入。若仍有问题请回复本邮件。",
    ],
    date: new Date(Date.now() - 50 * 3600_000).toISOString(),
    read: true,
    starred: false,
    tag: "工单",
  },
  {
    id: "m5",
    from: "运营平台",
    fromEmail: "ops@dante.cloud",
    title: "华东区租户新增授权应用提醒",
    preview: "租户「华东商贸集团」新授权了「数据分析」应用,涉及 4 个组织、12 个角色...",
    body: [
      "管理员您好:",
      "租户「华东商贸集团」刚刚完成了新的应用授权操作:数据分析(Athena)已授权给其下 4 个组织。",
      "相关角色的菜单权限范围将自动受组织授权约束。",
    ],
    date: new Date(Date.now() - 74 * 3600_000).toISOString(),
    read: true,
    starred: false,
    tag: "通知",
  },
]

const TAG_TONE: Record<MailItem["tag"], string> = {
  系统: "border-primary/25 bg-primary/10 text-primary",
  公告: "border-chart-4/25 bg-chart-4/10 text-chart-4",
  通知: "border-warning/30 bg-warning/10 text-warning",
  工单: "border-border bg-muted text-muted-foreground",
}

export default function InboxPage() {
  const [items, setItems] = React.useState<MailItem[]>(SEED)
  const [selectedId, setSelectedId] = React.useState<string | null>(null)
  const selected = items.find((i) => i.id === selectedId) ?? null
  const unread = items.filter((i) => !i.read).length

  const markRead = (id: string) => setItems((arr) => arr.map((i) => (i.id === id ? { ...i, read: true } : i)))
  const remove = (id: string) => {
    setItems((arr) => arr.filter((i) => i.id !== id))
    if (selectedId === id) setSelectedId(null)
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="收件箱"
        description="Hermes 消息中心 · 站内消息与通知(演示数据,本地状态)"
        icon={<InboxIcon className="size-5" />}
        badge={<DemoBadge />}
        actions={
          <Button variant="outline" size="sm" onClick={() => setItems((arr) => arr.map((i) => ({ ...i, read: true })))}>
            <CheckCheck className="size-4" />
            全部标为已读
          </Button>
        }
      />

      <div className="grid gap-4 lg:grid-cols-[360px_1fr]">
        {/* 左列表 */}
        <aside className="flex max-h-[36rem] min-h-[28rem] flex-col rounded-xl border bg-card">
          <div className="flex items-center justify-between border-b px-4 py-3">
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              消息({items.length})
            </p>
            {unread > 0 && <Badge className="text-[10px]">{unread} 未读</Badge>}
          </div>
          <ScrollArea className="flex-1">
            {items.length === 0 ? (
              <EmptyState className="py-14" title="收件箱是空的" description="收到新消息时会显示在这里" icon={<InboxIcon className="size-6" />} />
            ) : (
              <ul className="divide-y">
                {items.map((m) => (
                  <li key={m.id}>
                    <button
                      type="button"
                      onClick={() => {
                        setSelectedId(m.id)
                        markRead(m.id)
                      }}
                      className={cn(
                        "flex w-full items-start gap-3 px-4 py-3 text-left transition-colors",
                        m.id === selectedId ? "bg-primary/8" : "hover:bg-muted/50"
                      )}
                    >
                      <Avatar className="size-9 shrink-0 border">
                        <AvatarFallback className={cn("text-xs font-semibold", m.read ? "bg-muted text-muted-foreground" : "bg-primary/15 text-primary")}>
                          {m.from.slice(0, 1)}
                        </AvatarFallback>
                      </Avatar>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <p className={cn("truncate text-sm", m.read ? "text-muted-foreground" : "font-semibold")}>{m.title}</p>
                          {!m.read && <span className="size-2 shrink-0 rounded-full bg-primary" aria-label="未读" />}
                        </div>
                        <p className="mt-0.5 truncate text-xs text-muted-foreground">{m.from} · {formatRelative(m.date)}</p>
                        <p className="mt-1 line-clamp-1 text-xs text-muted-foreground/70">{m.preview}</p>
                      </div>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </ScrollArea>
        </aside>

        {/* 右详情 */}
        <section className="flex min-h-[28rem] flex-col rounded-xl border bg-card">
          {!selected ? (
            <EmptyState
              className="flex-1 justify-center py-14"
              title="选择一封消息"
              description="在左侧列表中选择消息查看全文"
              icon={<MailOpen className="size-6" />}
            />
          ) : (
            <motion.div key={selected.id} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.2 }} className="flex flex-1 flex-col">
              <div className="flex flex-col gap-3 border-b p-5 sm:flex-row sm:items-start sm:justify-between">
                <div className="flex items-start gap-3">
                  <Avatar className="size-10 border">
                    <AvatarFallback className="bg-primary/15 text-sm font-semibold text-primary">{selected.from.slice(0, 1)}</AvatarFallback>
                  </Avatar>
                  <div className="min-w-0">
                    <h2 className="text-base font-semibold leading-snug">{selected.title}</h2>
                    <p className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                      <span className="flex items-center gap-1">
                        <User className="size-3" />
                        {selected.from}
                      </span>
                      <code className="font-mono">{selected.fromEmail}</code>
                      <span className="flex items-center gap-1">
                        <MessageSquare className="size-3" />
                        {formatDateTime(selected.date)}
                      </span>
                    </p>
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  <span className={cn("rounded-md border px-2 py-0.5 text-xs", TAG_TONE[selected.tag])}>{selected.tag}</span>
                  <Button variant="ghost" size="sm" onClick={() => markRead(selected.id)} disabled={selected.read}>
                    <MailOpen className="size-4" />
                    标为已读
                  </Button>
                  <Button variant="ghost" size="sm" className="text-destructive hover:text-destructive" onClick={() => remove(selected.id)}>
                    <Trash2 className="size-4" />
                    删除
                  </Button>
                </div>
              </div>
              <div className="max-h-[36rem] flex-1 overflow-y-auto scroll-area p-5">
                <div className="mx-auto max-w-2xl space-y-4 text-sm leading-relaxed">
                  {selected.body.map((p, i) => (
                    <p key={i} className={i === 0 ? "font-medium" : "text-muted-foreground"}>
                      {p}
                    </p>
                  ))}
                </div>
              </div>
            </motion.div>
          )}
        </section>
      </div>
    </div>
  )
}
