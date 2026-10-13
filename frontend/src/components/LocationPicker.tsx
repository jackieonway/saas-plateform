import * as React from "react"
import * as L from "leaflet"
import "leaflet/dist/leaflet.css"
import { Loader2, LocateFixed, MapPin, Search, Wifi, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { resolvedLocation, type ResolvedLocation } from "@/lib/location"
import { regionsApi } from "@/api/regions"
import { toast } from "sonner"

export type MapPoint = { longitude: number; latitude: number }
export type MapSelection = MapPoint & { location?: ResolvedLocation }

type SearchResult = { display_name: string; lat: string; lon: string; address?: Record<string, unknown> }
type IpLocation = { success: boolean; latitude?: number; longitude?: number }

const DEFAULT_CENTER: L.LatLngTuple = [35.86, 104.20]
const TILE_URL = "/api/maps/tiles/{z}/{x}/{y}.png"
const TILE_ATTRIBUTION = import.meta.env.VITE_MAP_TILE_ATTRIBUTION || '&copy; <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noopener noreferrer">OpenStreetMap</a> contributors'

function validPoint(latitude: number, longitude: number): MapPoint | null {
  if (!Number.isFinite(latitude) || !Number.isFinite(longitude)
    || latitude < -90 || latitude > 90 || longitude < -180 || longitude > 180) return null
  return { latitude: Number(latitude.toFixed(6)), longitude: Number(longitude.toFixed(6)) }
}

function MapCanvas({
  initial,
  selected,
  onChoose,
  onReady,
}: {
  initial: MapPoint | null
  selected: MapPoint | null
  onChoose: (point: MapPoint) => void
  onReady: (map: L.Map | null) => void
}) {
  const containerRef = React.useRef<HTMLDivElement>(null)
  const mapRef = React.useRef<L.Map | null>(null)
  const markerRef = React.useRef<L.CircleMarker | null>(null)
  const onChooseRef = React.useRef(onChoose)
  onChooseRef.current = onChoose

  React.useEffect(() => {
    if (!containerRef.current) return
    const map = L.map(containerRef.current, { zoomControl: true, scrollWheelZoom: true })
    map.setView(initial ? [initial.latitude, initial.longitude] : DEFAULT_CENTER, initial ? 14 : 4)
    L.tileLayer(TILE_URL, { attribution: TILE_ATTRIBUTION, maxZoom: 19 }).addTo(map)
    map.on("click", (event: L.LeafletMouseEvent) => {
      onChooseRef.current({ latitude: Number(event.latlng.lat.toFixed(6)), longitude: Number(event.latlng.lng.toFixed(6)) })
    })
    mapRef.current = map
    onReady(map)
    const sizeTimer = window.setTimeout(() => map.invalidateSize(), 200)
    return () => {
      window.clearTimeout(sizeTimer)
      onReady(null)
      markerRef.current = null
      mapRef.current = null
      map.remove()
    }
    // The canvas is mounted once per dialog opening. Selection updates use the effect below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  React.useEffect(() => {
    const map = mapRef.current
    if (!map) return
    if (!selected) {
      markerRef.current?.remove()
      markerRef.current = null
      return
    }
    const point: L.LatLngExpression = [selected.latitude, selected.longitude]
    if (markerRef.current) markerRef.current.setLatLng(point)
    else markerRef.current = L.circleMarker(point, {
      radius: 9, color: "#fff", weight: 3, fillColor: "#e85d35", fillOpacity: 1,
    }).addTo(map)
  }, [selected])

  return <div ref={containerRef} className="h-[min(48vh,420px)] min-h-72 w-full rounded-lg bg-muted" aria-label="可点击选点的地图" />
}

export function LocationPicker({
  value,
  searchHint,
  regionVersion,
  onChange,
}: {
  value: MapPoint | null
  searchHint?: string
  regionVersion?: string | null
  onChange: (point: MapSelection | null) => void
}) {
  const [open, setOpen] = React.useState(false)
  const [draft, setDraft] = React.useState<MapPoint | null>(null)
  const [query, setQuery] = React.useState("")
  const [results, setResults] = React.useState<SearchResult[]>([])
  const [searchResultLocation, setSearchResultLocation] = React.useState<ResolvedLocation | null>(null)
  const [searching, setSearching] = React.useState(false)
  const [searchError, setSearchError] = React.useState("")
  const [locating, setLocating] = React.useState<"device" | "ip" | null>(null)
  const [locationInfo, setLocationInfo] = React.useState("")
  const [locationError, setLocationError] = React.useState("")
  const [locationSource, setLocationSource] = React.useState<"existing" | "device" | "ip" | "search" | "map">("existing")
  const [resolving, setResolving] = React.useState(false)
  const lastSearchAt = React.useRef(0)
  const mapRef = React.useRef<L.Map | null>(null)
  const locationRequest = React.useRef(0)
  const searchRequest = React.useRef(0)
  const searchController = React.useRef<AbortController | null>(null)
  const ipController = React.useRef<AbortController | null>(null)
  const reverseController = React.useRef<AbortController | null>(null)
  const regionController = React.useRef<AbortController | null>(null)

  React.useEffect(() => () => {
    locationRequest.current += 1
    searchRequest.current += 1
    searchController.current?.abort()
    ipController.current?.abort()
    reverseController.current?.abort()
    regionController.current?.abort()
  }, [])

  const cancelLocate = React.useCallback(() => {
    locationRequest.current += 1
    ipController.current?.abort()
    ipController.current = null
    reverseController.current?.abort()
    reverseController.current = null
    regionController.current?.abort()
    regionController.current = null
    setLocating(null)
    setResolving(false)
  }, [])

  React.useEffect(() => {
    if (reverseController.current || regionController.current) cancelLocate()
  }, [regionVersion, cancelLocate])

  const cancelSearch = () => {
    searchRequest.current += 1
    searchController.current?.abort()
    searchController.current = null
    setSearching(false)
  }

  const changeOpen = (next: boolean) => {
    if (!next) {
      cancelLocate()
      cancelSearch()
    }
    setOpen(next)
  }

  const openPicker = () => {
    setDraft(value)
    setQuery(searchHint?.trim() || "")
    setResults([])
    setSearchResultLocation(null)
    setSearchError("")
    setLocationError("")
    setLocationInfo(value ? "当前标记" : "")
    setLocationSource("existing")
    setResolving(false)
    changeOpen(true)
  }

  const applyLocation = (point: MapPoint, info: string, zoom: number, source: "device" | "ip" | "search") => {
    setDraft(point)
    setLocationInfo(info)
    setLocationSource(source)
    if (source !== "search") setSearchResultLocation(null)
    setLocationError("")
    mapRef.current?.flyTo([point.latitude, point.longitude], zoom)
  }

  const locateDevice = () => {
    if (!navigator.geolocation) {
      setLocationError("当前浏览器不支持设备定位，请使用 IP 估算或在地图上选点。")
      return
    }
    cancelLocate()
    const request = ++locationRequest.current
    setLocating("device")
    setLocationError("")
    try {
      navigator.geolocation.getCurrentPosition(
      (position) => {
        if (request !== locationRequest.current) return
        setLocating(null)
        const point = validPoint(position.coords.latitude, position.coords.longitude)
        if (!point) {
          setLocationError("设备返回的坐标无效，请在地图上选点。")
          return
        }
        const accuracy = Number.isFinite(position.coords.accuracy) ? ` · 约 ${Math.round(position.coords.accuracy)} 米` : ""
        applyLocation(point, `设备位置${accuracy}`, 16, "device")
      },
      (error) => {
        if (request !== locationRequest.current) return
        setLocating(null)
        setLocationError(error.code === 1
          ? "未获得浏览器位置权限；可选择 IP 估算或在地图上选点。"
          : "设备定位暂不可用；可选择 IP 估算或在地图上选点。")
      },
        { enableHighAccuracy: true, timeout: 12000, maximumAge: 60000 },
      )
    } catch {
      if (request === locationRequest.current) {
        setLocating(null)
        setLocationError("设备定位无法启动，请使用 IP 估算或在地图上选点。")
      }
    }
  }

  const locateIp = async () => {
    cancelLocate()
    const request = ++locationRequest.current
    const controller = new AbortController()
    ipController.current?.abort()
    ipController.current = controller
    setLocating("ip")
    setLocationError("")
    try {
      const response = await fetch("/api/maps/ip-location", { signal: controller.signal, cache: "no-store" })
      if (!response.ok) throw new Error(`HTTP ${response.status}`)
      const result = await response.json() as IpLocation
      const point = result.success ? validPoint(Number(result.latitude), Number(result.longitude)) : null
      if (!point) throw new Error("IP location unavailable")
      if (request !== locationRequest.current) return
      applyLocation(point, "IP 估算位置 · 城市级", 11, "ip")
    } catch {
      if (request === locationRequest.current) setLocationError("IP 定位暂不可用，请在地图上选点。")
    } finally {
      if (request === locationRequest.current) setLocating(null)
      if (ipController.current === controller) ipController.current = null
    }
  }

  const search = async () => {
    if (!query.trim() || searching) return
    if (Date.now() - lastSearchAt.current < 1000) {
      setSearchError("请稍等片刻再搜索。")
      return
    }
    lastSearchAt.current = Date.now()
    const request = ++searchRequest.current
    const controller = new AbortController()
    searchController.current?.abort()
    searchController.current = controller
    setSearching(true)
    setSearchError("")
    setResults([])
    try {
      const url = new URL("/api/maps/search", window.location.origin)
      url.searchParams.set("format", "jsonv2")
      url.searchParams.set("limit", "5")
      url.searchParams.set("addressdetails", "1")
      url.searchParams.set("q", query.trim())
      const response = await fetch(url, { signal: controller.signal, headers: { "Accept-Language": "zh-CN,zh;q=0.9" } })
      if (!response.ok) throw new Error(`HTTP ${response.status}`)
      const items = await response.json() as SearchResult[]
      if (request !== searchRequest.current) return
      setResults(items)
      if (items.length === 0) setSearchError("未找到地点，请换个关键词或直接在地图上选点。")
    } catch {
      if (request === searchRequest.current) setSearchError("地点搜索暂不可用，仍可拖动地图并点击选点。")
    } finally {
      if (request === searchRequest.current) setSearching(false)
      if (searchController.current === controller) searchController.current = null
    }
  }

  const chooseResult = (result: SearchResult) => {
    const point = validPoint(Number(result.lat), Number(result.lon))
    if (!point) return
    cancelLocate()
    cancelSearch()
    setSearchResultLocation(resolvedLocation(result))
    applyLocation(point, "搜索结果", 15, "search")
    setResults([])
  }

  const confirmSelection = async () => {
    if (!draft || resolving) return
    cancelLocate()
    cancelSearch()
    const request = locationRequest.current
    const point = draft
    const source = locationSource
    const searchLocation = source === "search" ? searchResultLocation : null
    const controller = new AbortController()
    reverseController.current = controller
    const timeout = window.setTimeout(() => controller.abort(), 8000)
    setResolving(true)
    let reverseLocation: ResolvedLocation = {}
    let reverseFailed = false
    try {
      const url = new URL("/api/maps/reverse", window.location.origin)
      url.searchParams.set("format", "jsonv2")
      url.searchParams.set("addressdetails", "1")
      url.searchParams.set("zoom", "18")
      url.searchParams.set("lat", String(point.latitude))
      url.searchParams.set("lon", String(point.longitude))
      const response = await fetch(url, { signal: controller.signal, headers: { "Accept-Language": "zh-CN,zh;q=0.9" } })
      if (!response.ok) throw new Error(`HTTP ${response.status}`)
      const data = await response.json() as unknown
      if (request !== locationRequest.current) return
      if (controller.signal.aborted || !data || typeof data !== "object" || "error" in data) throw new Error("Address unavailable")
      reverseLocation = resolvedLocation(data, source === "ip")
    } catch {
      if (request !== locationRequest.current) return
      reverseFailed = true
    } finally {
      window.clearTimeout(timeout)
      if (reverseController.current === controller) reverseController.current = null
    }
    if (request !== locationRequest.current) return

    const parsed: ResolvedLocation = {
      province: reverseLocation.province || searchLocation?.province,
      city: reverseLocation.city || searchLocation?.city,
      district: reverseLocation.district || searchLocation?.district,
      township: source === "ip" ? undefined : (reverseLocation.township || searchLocation?.township),
      village: source === "ip" ? undefined : (reverseLocation.village || searchLocation?.village),
      address: source === "ip" ? undefined : (reverseLocation.address || searchLocation?.address),
    }
    if (reverseFailed && !searchLocation) {
      onChange(point)
      toast.warning("坐标已回填，地址解析失败，请手动选择地区并填写地址")
      changeOpen(false)
      return
    }

    const resolver = new AbortController()
    regionController.current = resolver
    const regionTimeout = window.setTimeout(() => resolver.abort(), 8000)
    try {
      const regions = await regionsApi.resolve({
        version: regionVersion || undefined,
        province: parsed.province,
        city: parsed.city,
        district: parsed.district,
        township: parsed.township,
        village: parsed.village,
      }, { signal: resolver.signal })
      if (request !== locationRequest.current) return
      if (resolver.signal.aborted) throw new Error("Region matching timed out")
      onChange({ ...point, location: {
        province: regions.province?.name ?? "",
        city: regions.city?.name ?? "",
        district: regions.district?.name ?? "",
        township: source === "ip" ? "" : (regions.township?.name ?? ""),
        village: source === "ip" ? "" : (regions.village?.name ?? ""),
        address: parsed.address,
      } })
      if (!regions.province || (parsed.city && !regions.city) || (parsed.district && !regions.district)
        || (parsed.township && !regions.township) || (parsed.village && !regions.village)) {
        toast.warning("坐标和地址已回填，部分地区未能匹配地区管理数据，请手动选择")
      }
    } catch {
      if (request !== locationRequest.current) return
      onChange({ ...point, location: { address: parsed.address } })
      toast.warning("坐标和可用地址已回填，地区匹配失败，已保留原地区，请手动选择")
    } finally {
      window.clearTimeout(regionTimeout)
      if (regionController.current === resolver) regionController.current = null
      if (request === locationRequest.current) changeOpen(false)
    }
  }

  return <>
    <div className="flex flex-wrap items-center gap-2">
      <Button type="button" variant="outline" size="sm" onClick={openPicker}>
        <MapPin className="size-4" />在地图上选点
      </Button>
      {value ? <>
        <span className="text-xs tabular-nums text-muted-foreground">{value.longitude.toFixed(6)}, {value.latitude.toFixed(6)}</span>
        <Button type="button" variant="ghost" size="sm" className="h-8 px-2 text-muted-foreground" onClick={() => onChange(null)}>
          <X className="size-3.5" />清除
        </Button>
      </> : <span className="text-xs text-muted-foreground">尚未标注坐标</span>}
    </div>
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent className="max-h-[92vh] overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>在地图上选择位置</DialogTitle>
          <DialogDescription>搜索地点或拖动地图，再点击目标位置放置标记。确认后匹配已有地区并回填地址和坐标；坐标采用 WGS-84。</DialogDescription>
        </DialogHeader>
        <div className="flex gap-2">
          <Input value={query} onChange={(event) => { cancelSearch(); setQuery(event.target.value); setResults([]); setSearchError("") }}
            onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); void search() } }}
            placeholder="搜索省市、街道或地标" aria-label="搜索地图地点" />
          <Button type="button" variant="secondary" onClick={() => void search()} disabled={!query.trim() || searching}>
            {searching ? <Loader2 className="size-4 animate-spin" /> : <Search className="size-4" />}搜索
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">搜索会经本站地图代理查询 OpenStreetMap；也可以直接拖动地图选点。</p>
        {searchError && <p className="text-xs text-muted-foreground" role="status">{searchError}</p>}
        <div className="flex flex-wrap items-center gap-2 rounded-lg border bg-muted/40 p-2.5">
          <Button type="button" variant="outline" size="sm" onClick={locateDevice} disabled={locating !== null}>
            {locating === "device" ? <Loader2 className="size-4 animate-spin" /> : <LocateFixed className="size-4" />}
            定位当前设备
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={() => void locateIp()} disabled={locating !== null}>
            {locating === "ip" ? <Loader2 className="size-4 animate-spin" /> : <Wifi className="size-4" />}
            IP 估算位置
          </Button>
          <span className="text-xs text-muted-foreground">设备定位需浏览器授权；IP 定位会经本站代理向 ipwho.is 查询您的公网 IP，结果可能偏离实际位置。</span>
        </div>
        {locationError && <p className="text-xs text-destructive" role="status">{locationError}</p>}
        {results.length > 0 && <div className="max-h-36 overflow-y-auto rounded-lg border bg-card p-1" role="listbox" aria-label="地点搜索结果">
          {results.map((result, index) => <button key={`${result.lat}-${result.lon}-${index}`} type="button"
            className="block w-full rounded-md px-3 py-2 text-left text-xs hover:bg-muted focus-visible:bg-muted focus-visible:outline-none"
            onClick={() => chooseResult(result)}>{result.display_name}</button>)}
        </div>}
        {open && <div className="relative overflow-hidden rounded-lg border">
          <MapCanvas initial={value} selected={draft} onChoose={(point) => { cancelLocate(); cancelSearch(); setResults([]); setSearchResultLocation(null); setDraft(point); setLocationInfo("地图手选"); setLocationSource("map"); setLocationError("") }} onReady={(map) => { mapRef.current = map }} />
          <div className="pointer-events-none absolute bottom-3 left-3 z-[400] rounded-md bg-background/90 px-2.5 py-1.5 text-xs shadow-sm backdrop-blur-sm">
            {draft ? `${locationInfo || "选中"} · ${draft.longitude.toFixed(6)}, ${draft.latitude.toFixed(6)}` : "点击地图放置标记"}
          </div>
        </div>}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => changeOpen(false)}>取消</Button>
          <Button type="button" disabled={!draft || resolving} onClick={() => void confirmSelection()}>
            {resolving && <Loader2 className="size-4 animate-spin" />}{resolving ? "解析地址中" : "使用此位置"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </>
}
