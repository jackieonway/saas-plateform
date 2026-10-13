import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import { useFormContext, useWatch } from "react-hook-form"
import { regionsApi, type RegionRecord } from "@/api/regions"
import { Button } from "@/components/ui/button"
import { FormControl, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"

type RegionFields = { province: string; city: string; district: string; township: string; village: string }
const FIELDS = ["province", "city", "district", "township", "village"] as const
const LABELS = ["省份", "城市", "区县", "乡镇/街道", "村/社区"] as const

/** Shared catalog-backed fields for tenant and organization location forms. */
export function RegionPicker({ version, onVersionChange }: {
  version: string | null
  onVersionChange: (version: string | null) => void
}) {
  const form = useFormContext<RegionFields>()
  const versionId = React.useId()
  const [province, city, district, township, village] = useWatch({ control: form.control, name: FIELDS })
  const versions = useQuery({ queryKey: ["region-versions"], queryFn: regionsApi.versions })
  const provinces = useQuery({
    queryKey: ["regions", "options", version, "provinces"],
    queryFn: ({ signal }) => regionsApi.options({ version: version ?? undefined }, { signal }),
    retry: 1,
  })
  const activeVersion = version ?? provinces.data?.version ?? null

  React.useEffect(() => {
    if (!version && provinces.data?.version) onVersionChange(provinces.data.version)
  }, [version, provinces.data?.version, onVersionChange])

  const resolution = useQuery({
    queryKey: ["regions", "resolve", activeVersion, province, city, district, township, village],
    queryFn: ({ signal }) => regionsApi.resolve({ version: activeVersion!, province, city, district, township, village }, { signal }),
    enabled: Boolean(activeVersion && (province || city || district || township || village)),
    retry: 1,
  })
  const findSelected = (records: RegionRecord[] | undefined, name: string, matched: RegionRecord | null | undefined) =>
    name ? records?.find((record) => record.name === name || (matched && record.code === matched.code)) : undefined
  const selectedProvince = findSelected(provinces.data?.records, province, resolution.data?.province)
  const cities = useQuery({
    queryKey: ["regions", "options", activeVersion, selectedProvince?.code],
    queryFn: ({ signal }) => regionsApi.options({ version: activeVersion!, parentCode: selectedProvince!.code }, { signal }),
    enabled: Boolean(activeVersion && selectedProvince),
    retry: 1,
  })
  const selectedCity = findSelected(cities.data?.records, city, resolution.data?.city)
  const districts = useQuery({
    queryKey: ["regions", "options", activeVersion, selectedCity?.code],
    queryFn: ({ signal }) => regionsApi.options({ version: activeVersion!, parentCode: selectedCity!.code }, { signal }),
    enabled: Boolean(activeVersion && selectedCity),
    retry: 1,
  })
  const districtRecords = districts.data?.records.filter((record) => record.level === 4)
  const selectedDistrict = findSelected(districtRecords, district, resolution.data?.district)
  // Dongguan and Zhongshan have streets/towns directly under the city in the catalog.
  const townshipParent = selectedDistrict ?? (districts.isSuccess && districtRecords?.length === 0 ? selectedCity : undefined)
  const townships = useQuery({
    queryKey: ["regions", "options", activeVersion, townshipParent?.code],
    queryFn: ({ signal }) => regionsApi.options({ version: activeVersion!, parentCode: townshipParent!.code }, { signal }),
    enabled: Boolean(activeVersion && townshipParent),
    retry: 1,
  })
  const selectedTownship = findSelected(townships.data?.records.filter((record) => record.level === 5), township, resolution.data?.township)
  const villages = useQuery({
    queryKey: ["regions", "options", activeVersion, selectedTownship?.code],
    queryFn: ({ signal }) => regionsApi.options({ version: activeVersion!, parentCode: selectedTownship!.code }, { signal }),
    enabled: Boolean(activeVersion && selectedTownship),
    retry: 1,
  })
  const selectedVillage = findSelected(villages.data?.records, village, resolution.data?.village)

  // Hydrate legacy aliases (e.g. 北京 / 北京市) without marking an untouched form dirty.
  React.useEffect(() => {
    if (!resolution.data) return
    for (const key of FIELDS) {
      const matched = resolution.data[key]
      const current = form.getValues(key)
      if (current && matched && current !== matched.name) form.setValue(key, matched.name)
    }
  }, [resolution.data, form])

  const queries = [provinces, cities, districts, townships, villages] as const
  const selected = [selectedProvince, selectedCity, selectedDistrict, selectedTownship, selectedVillage]
  const enabled = [true, Boolean(selectedProvince), Boolean(selectedCity), Boolean(townshipParent), Boolean(selectedTownship)]
  const failed = queries.find((query, index) => enabled[index] && query.isError)
  const unresolved = resolution.isSuccess && FIELDS.some((key) => form.getValues(key) && !resolution.data[key])

  const choose = (index: number, code: string) => {
    const record = queries[index].data?.records.find((item) => item.code === code && item.level === index + 2)
    // Radix's hidden select can emit an empty value while async options register.
    // Only a catalog option or the explicit clear action may change form values.
    if (!record && code !== "__clear") return
    form.setValue(FIELDS[index], record?.name ?? "", { shouldDirty: true, shouldValidate: true })
    for (const key of FIELDS.slice(index + 1)) form.setValue(key, "", { shouldDirty: true, shouldValidate: true })
  }

  return <div className="space-y-2">
    <div className="flex items-center justify-between gap-3">
      <Label htmlFor={versionId} className="text-xs text-muted-foreground">地区数据版本</Label>
      <Select value={activeVersion ?? ""} disabled={!versions.data?.length} onValueChange={(next) => {
        if (next === activeVersion || !versions.data?.some((item) => item.version === next)) return
        onVersionChange(next)
        for (const key of FIELDS) form.setValue(key, "", { shouldDirty: true, shouldValidate: true })
      }}>
        <SelectTrigger id={versionId} aria-label="地区数据版本" className="h-8 max-w-44 text-xs">
          <SelectValue placeholder={versions.isLoading ? "加载中…" : "暂无版本"}>
            {versions.data?.find((item) => item.version === activeVersion)?.label ?? activeVersion}
          </SelectValue>
        </SelectTrigger>
        <SelectContent>{versions.data?.map((item) => <SelectItem key={item.version} value={item.version}>{item.label}</SelectItem>)}</SelectContent>
      </Select>
    </div>
    <div className="grid grid-cols-6 gap-2">
      {FIELDS.map((name, index) => <FormField key={name} control={form.control} name={name} render={({ field }) => {
        const query = queries[index]
        const records = query.data?.records.filter((record) => record.level === index + 2)
        const placeholder = query.isLoading ? "加载中…" : !enabled[index]
          ? `先选${LABELS[index - 1]}` : query.isError ? "加载失败"
          : index === 2 && records?.length === 0 ? "无区县层级" : "请选择"
        return <FormItem className={index < 3 ? "col-span-2 min-w-0" : "col-span-3 min-w-0"}>
          <FormLabel>{LABELS[index]}</FormLabel>
          <Select value={selected[index]?.code ?? (field.value ? "__legacy" : "")}
            disabled={!enabled[index] || query.isLoading || query.isError || !activeVersion || (index === 2 && records?.length === 0 && !field.value)}
            onValueChange={(code) => choose(index, code)}>
            <FormControl><SelectTrigger ref={field.ref} onBlur={field.onBlur} aria-label={LABELS[index]} className="w-full min-w-0">
              <SelectValue placeholder={placeholder}>{field.value || undefined}</SelectValue>
            </SelectTrigger></FormControl>
            <SelectContent>
              <SelectItem value="__clear">清除选择</SelectItem>
              {field.value && !selected[index] && <SelectItem value="__legacy" disabled>{field.value}（原值）</SelectItem>}
              {records?.map((record) => <SelectItem key={record.code} value={record.code}>{record.name}</SelectItem>)}
            </SelectContent>
          </Select>
          <FormMessage />
        </FormItem>
      }} />)}
    </div>
    {failed ? <div className="flex items-center gap-2 text-xs text-destructive" role="status">
      <span>{failed.error instanceof Error ? failed.error.message : "地区数据加载失败"}</span><Button type="button" variant="link" size="sm" className="h-auto p-0 text-xs" onClick={() => void failed.refetch()}>重试</Button>
    </div> : provinces.isSuccess && provinces.data.records.length === 0
      ? <p className="text-xs text-muted-foreground" role="status">当前版本暂无省份，请先在地区管理中维护数据。</p>
      : <p className="text-xs text-muted-foreground">省市县乡村与地图回填均使用地区管理中的数据；切换上级会清空全部下级。</p>}
    {unresolved && <p className="text-xs text-amber-600" role="status">原有地区尚未匹配当前版本，请重新选择；未调整的原值会保留。</p>}
    {resolution.isError && <div className="flex items-center gap-2 text-xs text-destructive" role="status">
      <span>原有地区匹配失败</span><Button type="button" variant="link" size="sm" className="h-auto p-0 text-xs" onClick={() => void resolution.refetch()}>重试</Button>
    </div>}
    {versions.isError && <div className="flex items-center gap-2 text-xs text-destructive" role="status">
      <span>地区版本加载失败</span><Button type="button" variant="link" size="sm" className="h-auto p-0 text-xs" onClick={() => void versions.refetch()}>重试</Button>
    </div>}
  </div>
}
