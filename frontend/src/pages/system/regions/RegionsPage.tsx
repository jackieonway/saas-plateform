import * as React from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { ChevronRight, MapPinned, Plus, Search } from "lucide-react"
import { toast } from "sonner"
import { PageHeader } from "@/components/layout/PageHeader"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { regionsApi, type RegionPayload, type RegionRecord } from "@/api/regions"

const REGION_LEVELS = [
  { value: 1, label: "国家" },
  { value: 2, label: "省级（省、自治区、直辖市）" },
  { value: 3, label: "地市级" },
  { value: 4, label: "区县级" },
  { value: 5, label: "乡镇、街道" },
  { value: 6, label: "村、社区" },
] as const

const ADMIN_TYPES = [
  { value: "COUNTRY", label: "国家", level: 1 },
  { value: "PROVINCE", label: "省", level: 2 },
  { value: "AUTON_REGION", label: "自治区", level: 2 },
  { value: "MUNICIPALITY", label: "直辖市", level: 2 },
  { value: "SAR", label: "特别行政区", level: 2 },
  { value: "PREF_CITY", label: "地级市", level: 3 },
  { value: "AUTON_PREF", label: "自治州", level: 3 },
  { value: "PREF_REGION", label: "地区", level: 3 },
  { value: "LEAGUE", label: "盟", level: 3 },
  { value: "DISTRICT", label: "市辖区", level: 4 },
  { value: "COUNTY_CITY", label: "县级市", level: 4 },
  { value: "COUNTY", label: "县", level: 4 },
  { value: "AUTON_COUNTY", label: "自治县", level: 4 },
  { value: "BANNER", label: "旗", level: 4 },
  { value: "AUTON_BANNER", label: "自治旗", level: 4 },
  { value: "SPECIAL_ZONE", label: "特区", level: 4 },
  { value: "FOREST_ZONE", label: "林区", level: 4 },
  { value: "STREET", label: "街道", level: 5 },
  { value: "TOWN", label: "镇", level: 5 },
  { value: "TOWNSHIP", label: "乡", level: 5 },
  { value: "ETHNIC_TOWN", label: "民族乡", level: 5 },
  { value: "SUMU", label: "苏木", level: 5 },
  { value: "ETHNIC_SUMU", label: "民族苏木", level: 5 },
  { value: "VILLAGE", label: "村委会", level: 6 },
  { value: "COMMUNITY", label: "居委会、社区", level: 6 },
  { value: "OTHER", label: "其他", level: 6 },
] as const

const RURAL_TYPES = [
  { value: "111", label: "主城区" },
  { value: "112", label: "城乡结合区" },
  { value: "121", label: "镇中心区" },
  { value: "122", label: "镇乡结合区" },
  { value: "123", label: "特殊区域" },
  { value: "210", label: "乡中心区" },
  { value: "220", label: "村庄" },
] as const
const REGION_TYPES = [...ADMIN_TYPES, ...RURAL_TYPES]

const levelLabel = (level: number) => REGION_LEVELS.find(item => item.value === level)?.label ?? `第 ${level} 级`
const typeLabel = (type: string | null, typeName: string | null) =>
  REGION_TYPES.find(item => item.value === type)?.label ?? typeName ?? "未设置"

const empty = (version: string, parentCode = "-1", level = 1): RegionPayload => ({
  version, code: "", parentCode, name: "", namePinyin: "", nameShort: "", level,
  type: null, typeName: null, postCode: null, sourceUrl: null,
})

export default function RegionsPage() {
  const client = useQueryClient()
  const versions = useQuery({ queryKey: ["region-versions"], queryFn: regionsApi.versions })
  const [version, setVersion] = React.useState("2023")
  const [parent, setParent] = React.useState<RegionRecord | null>(null)
  const [history, setHistory] = React.useState<RegionRecord[]>([])
  const [search, setSearch] = React.useState("")
  const [term, setTerm] = React.useState("")
  const [page, setPage] = React.useState(1)
  const [editing, setEditing] = React.useState<RegionRecord | null>(null)
  const [form, setForm] = React.useState<RegionPayload | null>(null)
  const [saving, setSaving] = React.useState(false)
  const [newVersion, setNewVersion] = React.useState(false)
  const [versionId, setVersionId] = React.useState("")
  const [copyFrom, setCopyFrom] = React.useState("")
  React.useEffect(() => {
    if (versions.data && !versions.data.some(item => item.version === version)) {
      setVersion(versions.data[0]?.version ?? "")
      setParent(null); setHistory([]); setPage(1)
    }
  }, [version, versions.data])
  const rows = useQuery({
    queryKey: ["regions", version, parent?.code, term, page],
    queryFn: () => regionsApi.list({ version, parentCode: parent ? (parent.level === 1 ? "0" : parent.code) : "-1", search: term || undefined, page, size: 50 }),
    enabled: !!version,
  })
  const refresh = () => Promise.all([
    client.invalidateQueries({ queryKey: ["regions"] }),
    client.invalidateQueries({ queryKey: ["region-versions"] }),
  ])
  const navigate = (node: RegionRecord) => { setHistory([...history, node]); setParent(node); setTerm(""); setSearch(""); setPage(1) }
  const back = (index: number) => { const path = history.slice(0, index + 1); setHistory(path); setParent(path.at(-1) ?? null); setPage(1) }
  const save = async () => {
    if (!form) return
    setSaving(true)
    try {
      if (editing) await regionsApi.update(editing.id, form)
      else await regionsApi.create(form)
      toast.success("地区已保存")
      setForm(null); refresh()
    } catch (e) { toast.error((e as Error).message) }
    finally { setSaving(false) }
  }
  const remove = async (node: RegionRecord) => {
    if (!window.confirm(`确认删除「${node.name}」？存在下级地区时无法删除。`)) return
    try { await regionsApi.remove(version, node.id); toast.success("已删除"); refresh() }
    catch (e) { toast.error((e as Error).message) }
  }
  const createVersion = async () => {
    try {
      await regionsApi.createVersion({version:versionId,label:`${versionId}版`,sourceVersion:copyFrom || undefined})
      await refresh()
      setVersion(versionId); setVersionId(""); setNewVersion(false); setParent(null); setHistory([])
      toast.success("版本已创建")
    } catch (e) { toast.error((e as Error).message) }
  }
  const deleteVersion = async () => {
    const selected = versions.data?.find(item => item.version === version)
    if (!selected || !window.confirm(`确认删除「${selected.label}」及其 ${selected.count.toLocaleString()} 条地区数据？此操作无法撤销。`)) return
    try {
      await regionsApi.removeVersion(version)
      setVersion(versions.data?.find(item => item.version !== version)?.version ?? "")
      setParent(null); setHistory([]); setPage(1); refresh()
      toast.success("版本及地区数据已删除")
    } catch (e) { toast.error((e as Error).message) }
  }
  return <div className="space-y-5">
    <PageHeader title="地区管理" description="按数据版本维护行政区划；逐级加载地区数据" icon={<MapPinned className="size-5" />}
      actions={<Button onClick={() => { setNewVersion(true); setCopyFrom(version) }}><Plus className="mr-2 size-4" />新建版本</Button>} />
    <div className="rounded-xl border bg-card p-4 space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <label className="text-sm font-medium">数据版本</label>
        <select className="h-9 rounded-md border bg-background px-3 text-sm" value={version} onChange={e => { setVersion(e.target.value); setParent(null); setHistory([]); setPage(1) }}>
          {(versions.data ?? []).map(v => <option key={v.version} value={v.version}>{v.label} · {v.count.toLocaleString()} 条</option>)}
        </select>
        <Button variant="outline" size="sm" className="text-destructive" disabled={!version} onClick={deleteVersion}>删除版本</Button>
        <form className="ml-auto flex gap-2" onSubmit={e => { e.preventDefault(); setTerm(search.trim()); setPage(1) }}>
          <Input className="w-64" value={search} onChange={e => setSearch(e.target.value)} placeholder="搜索编码、名称或拼音" />
          <Button variant="outline" type="submit"><Search className="size-4" /></Button>
        </form>
      </div>
      <div className="flex flex-wrap items-center gap-1 text-sm text-muted-foreground">
        <button className="hover:text-primary" onClick={() => { setHistory([]); setParent(null); setPage(1) }}>全部地区</button>
        {history.map((item, i) => <React.Fragment key={item.id}><ChevronRight className="size-3" /><button className="hover:text-primary" onClick={() => back(i)}>{item.name}</button></React.Fragment>)}
        {term && <span> · 搜索“{term}”</span>}
      </div>
      <div className="flex justify-end"><Button size="sm" disabled={!version || (parent !== null && parent.level >= 6)} onClick={() => { setEditing(null); setForm(empty(version,parent ? (parent.level === 1 ? "0" : parent.code) : "-1",(parent?.level ?? 0)+1)) }}><Plus className="mr-1 size-4" />新增地区</Button></div>
      {rows.isLoading ? <p className="py-12 text-center text-muted-foreground">加载中…</p> : rows.isError ? <p className="py-12 text-center text-destructive">{(rows.error as Error).message}</p> : <>
        <div className="overflow-x-auto"><table className="w-full text-sm"><thead className="border-b text-left text-muted-foreground"><tr><th className="p-3">地区</th><th className="p-3">编码</th><th className="p-3">层级</th><th className="p-3">类型</th><th className="p-3">拼音</th><th className="p-3 text-right">操作</th></tr></thead><tbody>
          {(rows.data?.records ?? []).map(r => <tr key={r.id} className="border-b last:border-0 hover:bg-muted/40"><td className="p-3"><button className="font-medium text-primary hover:underline" onClick={() => navigate(r)}>{r.name} <ChevronRight className="inline size-3" /></button></td><td className="p-3 font-mono">{r.code}</td><td className="p-3">{levelLabel(r.level)}</td><td className="p-3">{typeLabel(r.type, r.typeName)}</td><td className="p-3">{r.namePinyin}</td><td className="p-3 text-right whitespace-nowrap"><Button variant="ghost" size="sm" onClick={() => { setEditing(r); setForm({...r}) }}>编辑</Button><Button variant="ghost" size="sm" className="text-destructive" onClick={() => remove(r)}>删除</Button></td></tr>)}
        </tbody></table></div>
        {!rows.data?.records.length && <p className="py-10 text-center text-muted-foreground">暂无地区数据</p>}
        <div className="flex items-center justify-end gap-3 text-sm text-muted-foreground"><span>共 {rows.data?.total ?? 0} 条</span><Button variant="outline" size="sm" disabled={page<=1} onClick={() => setPage(page-1)}>上一页</Button><span>第 {page} 页</span><Button variant="outline" size="sm" disabled={page*50 >= (rows.data?.total ?? 0)} onClick={() => setPage(page+1)}>下一页</Button></div>
      </>}
    </div>
    {newVersion && <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"><div className="w-full max-w-md space-y-4 rounded-xl bg-background p-6 shadow-xl"><h2 className="font-semibold">新建数据版本</h2><Input value={versionId} onChange={e => setVersionId(e.target.value)} placeholder="版本标识，如 2024" /><select className="h-9 w-full rounded-md border bg-background px-3 text-sm" value={copyFrom} onChange={e => setCopyFrom(e.target.value)}><option value="">空白版本</option>{(versions.data ?? []).map(v => <option key={v.version} value={v.version}>复制 {v.label}</option>)}</select><p className="text-xs text-muted-foreground">复制后可逐条维护；版本标签表示管理版本，不代表来源数据已更新。</p><div className="flex justify-end gap-2"><Button variant="outline" onClick={() => setNewVersion(false)}>取消</Button><Button disabled={!versionId.trim()} onClick={createVersion}>创建</Button></div></div></div>}
    {form && <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"><div className="max-h-[90vh] w-full max-w-xl overflow-y-auto space-y-4 rounded-xl bg-background p-6 shadow-xl"><h2 className="font-semibold">{editing ? "编辑地区" : "新增地区"} · {version}版</h2><div className="grid grid-cols-2 gap-3">
      {([ ["code","地区编码"],["parentCode","父级编码"],["name","地区名称"],["namePinyin","地区拼音"],["nameShort","拼音简称"] ] as const).map(([key,label]) => <label key={key} className="space-y-1 text-sm"><span>{label}</span><Input value={form[key]} onChange={e => setForm({...form,[key]:e.target.value})} /></label>)}
      <label className="space-y-1 text-sm"><span>层级</span><select className="h-9 w-full rounded-md border bg-background px-3" value={form.level} onChange={e => { const level = Number(e.target.value); const selected = ADMIN_TYPES.find(item => item.value === form.type); const keepType = selected?.level === level || (level === 6 && RURAL_TYPES.some(item => item.value === form.type)); setForm({...form,level,type:keepType ? form.type : null,typeName:keepType ? form.typeName : null}) }}>{REGION_LEVELS.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}</select></label>
      <label className="space-y-1 text-sm"><span>地区类型</span><select className="h-9 w-full rounded-md border bg-background px-3" value={form.type ?? ""} onChange={e => { const selected = REGION_TYPES.find(item => item.value === e.target.value); setForm({...form,type:selected?.value ?? null,typeName:selected?.label ?? null}) }}><option value="">未设置</option><optgroup label="行政区划类型">{ADMIN_TYPES.filter(item => item.level === form.level).map(item => <option key={item.value} value={item.value}>{item.label}</option>)}</optgroup>{form.level === 6 && <optgroup label="城乡分类（原始数据）">{RURAL_TYPES.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}</optgroup>}</select></label>
      <label className="space-y-1 text-sm"><span>邮编</span><Input value={form.postCode ?? ""} onChange={e => setForm({...form,postCode:e.target.value || null})} /></label>
      <label className="col-span-2 space-y-1 text-sm"><span>来源链接</span><Input value={form.sourceUrl ?? ""} onChange={e => setForm({...form,sourceUrl:e.target.value || null})} /></label>
    </div><div className="flex justify-end gap-2"><Button variant="outline" onClick={() => setForm(null)}>取消</Button><Button disabled={saving} onClick={save}>{saving ? "保存中…" : "保存"}</Button></div></div></div>}
  </div>
}
