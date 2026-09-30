import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import {
  Building2,
  CalendarClock,
  KeyRound,
  Mail,
  Pencil,
  Phone,
  Shield,
  ShieldCheck,
  UserRound,
  UsersRound,
  VenusAndMars,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { Skeleton } from "@/components/ui/skeleton"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { StatusBadge } from "@/components/StatusBadge"
import { CopyButton } from "@/components/CopyButton"
import { usersApi } from "@/api/users"
import type { UserRecord } from "@/api/types"
import { formatDateTime, formatRelative, GENDER_LABELS } from "@/lib/format"
import { cn } from "@/lib/utils"

/** GET /api/users/:id/memberships 返回行 */
interface MembershipRow {
  organizationId: string
  organizationName: string
  departmentId: string | null
  departmentName: string | null
  position: string
  tenantId: string
}

function genderLabelSafe(g: string): string {
  return GENDER_LABELS[g] ?? "未知"
}

/** GET /api/users/:id/roles 返回行 (增强版带名称) */
interface RoleRow {
  roleId: string
  organizationId: string
  roleName: string
  roleCode: string
  organizationName: string
}

export function UserDetailDrawer({
  user,
  onOpenChange,
  onEdit,
  onMemberships,
  onRoles,
  onResetPassword,
}: {
  user: UserRecord | null
  onOpenChange: (o: boolean) => void
  onEdit: (u: UserRecord) => void
  onMemberships: (u: UserRecord) => void
  onRoles: (u: UserRecord) => void
  onResetPassword: (u: UserRecord) => void
}) {
  const open = !!user
  const uid = user?.id

  const membershipsQuery = useQuery({
    queryKey: ["user-detail", uid, "memberships"],
    queryFn: () => usersApi.memberships(uid!) as Promise<MembershipRow[]>,
    enabled: open && !!uid,
    staleTime: 15_000,
    retry: 1,
  })
  const rolesQuery = useQuery({
    queryKey: ["user-detail", uid, "roles"],
    queryFn: () => usersApi.roles(uid!) as Promise<RoleRow[]>,
    enabled: open && !!uid,
    staleTime: 15_000,
    retry: 1,
  })

  if (!user) return null
  const memberships = membershipsQuery.data ?? []
  const roles = rolesQuery.data ?? []

  const initials = (user.realName || user.username || "?").slice(0, 1).toUpperCase()

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="flex w-full gap-0 p-0 sm:max-w-md">
        <SheetHeader className="border-b pb-5">
          <div className="flex items-start justify-between gap-3">
            <div className="flex min-w-0 items-center gap-4">
              <div className="relative">
                <div
                  aria-hidden
                  className="flex size-14 items-center justify-center rounded-2xl bg-gradient-to-br from-primary/25 to-primary/5 text-xl font-bold text-primary"
                >
                  {initials}
                </div>
                <span
                  aria-hidden
                  className={cn(
                    "absolute -bottom-0.5 -right-0.5 size-3.5 rounded-full border-2 border-background",
                    user.status === "ENABLED" ? "bg-emerald-500" : "bg-muted-foreground/50"
                  )}
                />
              </div>
              <div className="min-w-0">
                <SheetTitle className="truncate text-lg">{user.realName || user.username}</SheetTitle>
                <p className="mt-0.5 truncate font-mono text-xs text-muted-foreground">
                  @{user.username}
                </p>
                <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                  <StatusBadge status={user.status} />
                  {user.gender && user.gender !== "UNKNOWN" && (
                    <Badge variant="outline" className="gap-1 text-[11px] font-normal">
                      <VenusAndMars className="size-3" />
                      {genderLabelSafe(user.gender)}
                    </Badge>
                  )}
                </div>
              </div>
            </div>
          </div>
          <SheetDescription className="sr-only">用户详细信息</SheetDescription>
          <div className="mt-1 flex flex-wrap gap-2 pt-1">
            <Button variant="outline" size="sm" onClick={() => onMemberships(user)}>
              <UsersRound className="size-4" />
              成员关系
            </Button>
            <Button variant="outline" size="sm" onClick={() => onRoles(user)}>
              <Shield className="size-4" />
              分配角色
            </Button>
            <Button variant="outline" size="sm" onClick={() => onResetPassword(user)}>
              <KeyRound className="size-4" />
              重置密码
            </Button>
            <Button size="sm" onClick={() => onEdit(user)}>
              <Pencil className="size-4" />
              编辑
            </Button>
          </div>
        </SheetHeader>

        <div className="flex-1 space-y-6 overflow-y-auto scroll-area p-5">
          {/* 基本信息 */}
          <section aria-label="基本信息">
            <h3 className="mb-3 flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              <UserRound className="size-3.5" />
              基本信息
            </h3>
            <dl className="grid grid-cols-2 gap-x-4 gap-y-3.5">
              <InfoItem icon={<Mail className="size-3.5" />} label="邮箱" value={user.email || "—"} mono copyable />
              <InfoItem icon={<Phone className="size-3.5" />} label="手机号" value={user.phone || "—"} mono copyable />
              <InfoItem
                icon={<ShieldCheck className="size-3.5" />}
                label="角色数"
                value={String(user.roleCount)}
              />
              <InfoItem
                icon={<CalendarClock className="size-3.5" />}
                label="创建时间"
                value={formatRelative(user.createdAt)}
                title={formatDateTime(user.createdAt)}
              />
              <InfoItem
                icon={<CalendarClock className="size-3.5" />}
                label="最后登录"
                value={user.lastLoginAt ? formatRelative(user.lastLoginAt) : "从未登录"}
                title={user.lastLoginAt ? formatDateTime(user.lastLoginAt) : undefined}
              />
              <InfoItem
                icon={<UserRound className="size-3.5" />}
                label="用户 ID"
                value={user.id}
                mono
                copyable
              />
            </dl>
          </section>

          <Separator />

          {/* 成员关系 */}
          <section aria-label="成员关系">
            <div className="mb-3 flex items-center justify-between">
              <h3 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                <Building2 className="size-3.5" />
                成员关系
              </h3>
              <span className="text-xs tabular-nums text-muted-foreground">{memberships.length} 项</span>
            </div>
            {membershipsQuery.isLoading ? (
              <div className="space-y-2">
                {Array.from({ length: 2 }).map((_, i) => (
                  <Skeleton key={i} className="h-16 w-full rounded-xl" />
                ))}
              </div>
            ) : memberships.length === 0 ? (
              <p className="rounded-xl border border-dashed p-4 text-center text-xs text-muted-foreground">
                暂未加入任何组织
              </p>
            ) : (
              <ul className="space-y-2">
                {memberships.map((m, i) => (
                  <li key={`${m.organizationId}-${i}`} className="rounded-xl border bg-card p-3">
                    <div className="flex items-center justify-between gap-2">
                      <p className="flex min-w-0 items-center gap-2 text-sm font-medium">
                        <Building2 className="size-4 shrink-0 text-primary/70" />
                        <span className="truncate">{m.organizationName}</span>
                      </p>
                      {m.position && (
                        <Badge variant="secondary" className="shrink-0 text-[11px] font-normal">
                          {m.position}
                        </Badge>
                      )}
                    </div>
                    <p className="mt-1.5 pl-6 text-xs text-muted-foreground">
                      {m.departmentName ? (
                        <span className="inline-flex items-center gap-1">
                          <span className="size-1 rounded-full bg-primary/50" aria-hidden />
                          {m.departmentName}
                        </span>
                      ) : (
                        "未分配部门"
                      )}
                    </p>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <Separator />

          {/* 角色 */}
          <section aria-label="角色">
            <div className="mb-3 flex items-center justify-between">
              <h3 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                <Shield className="size-3.5" />
                拥有角色
              </h3>
              <span className="text-xs tabular-nums text-muted-foreground">{roles.length} 项</span>
            </div>
            {rolesQuery.isLoading ? (
              <div className="space-y-2">
                {Array.from({ length: 2 }).map((_, i) => (
                  <Skeleton key={i} className="h-14 w-full rounded-xl" />
                ))}
              </div>
            ) : roles.length === 0 ? (
              <p className="rounded-xl border border-dashed p-4 text-center text-xs text-muted-foreground">
                暂未分配任何角色
              </p>
            ) : (
              <ul className="space-y-2">
                {roles.map((r, i) => (
                  <li
                    key={`${r.roleId}-${i}`}
                    className="flex items-center justify-between gap-3 rounded-xl border bg-card p-3"
                  >
                    <div className="flex min-w-0 items-center gap-2.5">
                      <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                        <ShieldCheck className="size-4" />
                      </span>
                      <div className="min-w-0">
                        <p className="truncate text-sm font-medium">{r.roleName || r.roleId}</p>
                        <p className="truncate font-mono text-[11px] text-muted-foreground">
                          {r.roleCode || "—"}
                        </p>
                      </div>
                    </div>
                    {r.organizationName && (
                      <Badge variant="secondary" className="shrink-0 max-w-[8rem] truncate text-[11px] font-normal">
                        {r.organizationName}
                      </Badge>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>
      </SheetContent>
    </Sheet>
  )
}

function InfoItem({
  icon,
  label,
  value,
  mono,
  copyable,
  full,
  title,
}: {
  icon: React.ReactNode
  label: string
  value: string
  mono?: boolean
  copyable?: boolean
  full?: boolean
  title?: string
}) {
  return (
    <div className={cn("min-w-0", full && "col-span-2")}>
      <dt className="flex items-center gap-1 text-[11px] text-muted-foreground">
        {icon}
        {label}
      </dt>
      <dd
        title={title ?? value}
        className={cn(
          "mt-1 flex min-w-0 items-center gap-1",
          value === "—" && "text-muted-foreground/60"
        )}
      >
        {copyable && value !== "—" ? (
          <>
            <span className={cn("truncate text-sm select-all", mono && "font-mono text-[13px]")}>
              {value}
            </span>
            <CopyButton
              text={value}
              size="icon"
              className="size-5 shrink-0 border-transparent bg-transparent p-0 text-muted-foreground/50 opacity-0 shadow-none transition-opacity hover:text-foreground focus-visible:opacity-100 [dd:hover>&]:opacity-100"
            />
          </>
        ) : (
          <span className={cn("truncate text-sm", mono && "font-mono text-[13px]")}>{value}</span>
        )}
      </dd>
    </div>
  )
}
