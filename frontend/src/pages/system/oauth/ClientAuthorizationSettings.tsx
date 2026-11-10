import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import type { ClientPolicyDraft, TokenField } from "./client-policy"

export function ClientAuthorizationSettings({ machine, editing, firstParty, value, onChange, errors, defaults }: {
  machine: boolean
  editing: boolean
  firstParty: boolean
  value: ClientPolicyDraft
  onChange: (field: keyof ClientPolicyDraft, value: boolean | string) => void
  errors?: Partial<Record<TokenField, string>>
  defaults?: { accessTokenTtlSeconds: number; refreshTokenTtlSeconds: number; clientTokenTtlSeconds: number }
}) {
  const ttl = (field: TokenField, label: string, refresh = false) => <div className="space-y-2" key={field}>
    <Label htmlFor={field}>{label}</Label>
    <div className="flex items-center gap-2">
      <Select value={value[field] === "" ? "INHERIT" : "CUSTOM"} onValueChange={mode => onChange(field, mode === "INHERIT" ? "" : String(defaults?.[field] ?? (refresh ? 1800 : 300)))}>
        <SelectTrigger aria-label={`${label}配置方式`} className="w-36 shrink-0"><SelectValue>{value[field] === "" ? "继承全局配置" : "自定义"}</SelectValue></SelectTrigger>
        <SelectContent><SelectItem value="INHERIT">继承全局配置</SelectItem><SelectItem value="CUSTOM">自定义</SelectItem></SelectContent>
      </Select>
      {value[field] !== "" && <><Input id={field} aria-label={label} inputMode="numeric" type="number" min={refresh ? 300 : 60} max={refresh ? 604800 : 3600} step={1} value={value[field]} onChange={e => onChange(field, e.target.value)} className="font-mono" /><span className="text-xs text-muted-foreground">秒</span></>}
      {value[field] === "" && defaults && <span className="font-mono text-xs text-muted-foreground">{defaults[field]} 秒</span>}
    </div>
    <p className="text-xs text-muted-foreground">{refresh ? "自定义范围 300～604800 秒；实际期限受原登录来源和最长续期边界限制。" : "自定义范围 60～3600 秒；实际期限不超过原授权或客户端凭据的剩余期限。"}</p>
    {errors?.[field] && <p role="alert" className="text-xs text-destructive">{errors[field]}</p>}
  </div>

  return <>
    <section className="space-y-4 rounded-xl border p-4" aria-label="授权设置">
      <h3 className="text-sm font-semibold">授权设置</h3>
      <div className="space-y-2"><Label>允许的授权类型</Label>
        <div className="flex flex-wrap gap-4 rounded-lg bg-muted/25 p-3">
          <label className="flex items-center gap-2 text-sm"><Checkbox checked={!machine} disabled />授权码（PKCE）</label>
          <label className="flex items-center gap-2 text-sm"><Checkbox checked={!machine && value.refreshToken} disabled={machine || editing} onCheckedChange={checked => { onChange("refreshToken", checked === true); if (checked !== true) onChange("refreshTokenTtlSeconds", "") }} />刷新令牌</label>
          <label className="flex items-center gap-2 text-sm"><Checkbox checked={machine} disabled />客户端凭据</label>
        </div>
        <p className="text-xs text-muted-foreground">授权码用于用户登录，客户端凭据用于服务间调用；用途及授权类型创建后固定。当前不支持隐式和密码模式。</p>
      </div>
      {!machine && <div className="space-y-2"><Label>授权确认方式</Label>
        <Select value={value.autoAuthorize ? "AUTO" : "MANUAL"} onValueChange={mode => onChange("autoAuthorize", mode === "AUTO")}>
          <SelectTrigger aria-label="授权确认方式"><SelectValue>{value.autoAuthorize ? "自动授权 · 可信自有客户端" : "用户确认授权（默认）"}</SelectValue></SelectTrigger>
          <SelectContent><SelectItem value="MANUAL">用户确认授权（默认）</SelectItem><SelectItem value="AUTO" disabled={!firstParty}>自动授权 · 可信自有客户端</SelectItem></SelectContent>
        </Select>
        <p className="text-xs leading-5 text-muted-foreground">自动授权只省去授权确认，仍须登录及选择员工身份。第三方、离线访问、用户已撤销授权或明确要求重新确认时，仍需用户确认。</p>
      </div>}
    </section>
    <section className="space-y-5 rounded-xl border p-4" aria-label="令牌设置">
      <h3 className="text-sm font-semibold">令牌设置</h3>
      {machine ? ttl("clientTokenTtlSeconds", "客户端令牌有效期") : <>
        {ttl("accessTokenTtlSeconds", "访问令牌有效期")}
        {value.refreshToken && <>
          {ttl("refreshTokenTtlSeconds", "刷新令牌空闲有效期", true)}
          <div className="rounded-lg bg-muted/25 p-3"><p className="text-sm font-medium">刷新令牌策略：轮换</p><p className="mt-1 text-xs leading-5 text-muted-foreground">每次刷新签发新令牌，旧令牌再次使用时拒绝请求。不支持延用旧令牌。</p></div>
        </>}
      </>}
    </section>
  </>
}
