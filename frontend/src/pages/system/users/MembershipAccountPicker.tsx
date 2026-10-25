import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import { Check, Search, X } from "lucide-react"
import { usersApi } from "@/api/users"
import type { UserRecord } from "@/api/types"
import { asPage } from "@/lib/api"
import { useDebounced } from "@/lib/format"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ErrorState } from "@/components/states"

export interface SelectedMembershipAccount {
  id: string
  username: string
  realName: string
}

/** A directory result must be explicitly selected. Matching contact details
 * never select an account or merge two directory records. */
export function MembershipAccountPicker({ namespace, value, onChange, disabled = false }: {
  namespace: string | null
  value: SelectedMembershipAccount | null
  onChange: (value: SelectedMembershipAccount | null) => void
  disabled?: boolean
}) {
  const [keyword, setKeyword] = React.useState("")
  const debounced = useDebounced(keyword.trim(), 250)
  const query = useQuery({
    queryKey: ["membership-account-candidates", namespace, debounced],
    queryFn: async () => asPage<UserRecord>(await usersApi.list({ keyword: debounced, page: 1, size: 20 })),
    enabled: !!namespace && debounced.length >= 2 && !value,
    retry: 1,
  })

  if (value) return (
    <div className="rounded-lg border bg-muted/30 p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <p className="flex items-center gap-1.5 text-sm font-medium"><Check className="size-3.5 text-emerald-600" />{value.realName || value.username}</p>
          <p className="text-xs text-muted-foreground">@{value.username}</p>
          <p className="break-all font-mono text-xs" data-testid="selected-account-id">{value.id}</p>
        </div>
        <Button type="button" variant="ghost" size="icon" className="size-8" disabled={disabled} onClick={() => onChange(null)} aria-label="重新选择绑定账号"><X className="size-4" /></Button>
      </div>
    </div>
  )

  return (
    <div className="space-y-2">
      <div className="relative">
        <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input aria-label="检索既有账号" className="pl-9" placeholder="输入用户名或姓名，再选择准确账号" value={keyword} onChange={event => setKeyword(event.target.value)} disabled={disabled} />
      </div>
      <p className="text-xs text-muted-foreground">从当前可管理目录中选择已有账号。相同姓名、邮箱或电话不代表同一账号，请核对下方完整账号 ID。</p>
      {query.isError && <ErrorState message="账号目录加载失败，无法确认绑定对象" onRetry={() => query.refetch()} />}
      {debounced.length >= 2 && !query.isError && (
        <div className="max-h-44 overflow-y-auto rounded-lg border">
          {query.isFetching ? <p className="p-3 text-xs text-muted-foreground">正在检索可管理账号…</p> : query.data?.records.length ? query.data.records.map(account => (
            <button type="button" key={account.id} className="block w-full space-y-1 border-b px-3 py-2.5 text-left last:border-b-0 hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:outline-none" disabled={disabled} onClick={() => onChange({ id: account.id, username: account.username, realName: account.realName })}>
              <p className="text-sm font-medium">{account.realName || account.username}<span className="ml-2 text-xs font-normal text-muted-foreground">@{account.username}</span></p>
              <p className="break-all font-mono text-[11px] text-muted-foreground">{account.id}</p>
            </button>
          )) : <p className="p-3 text-xs text-muted-foreground">没有匹配的可管理账号，请调整检索条件。</p>}
          {!!query.data && query.data.total > query.data.records.length && <p className="border-t p-2 text-xs text-muted-foreground">当前显示前 20 项，请缩小检索范围后准确选择。</p>}
        </div>
      )}
    </div>
  )
}
