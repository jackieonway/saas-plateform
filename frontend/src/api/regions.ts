import { api, qs } from "@/lib/api"

export interface RegionVersion { version: string; label: string; sourceVersion: string | null; count: number }
export interface RegionRecord {
  id: string; version: string; code: string; parentCode: string; name: string;
  namePinyin: string; nameShort: string; level: number; type: string | null;
  typeName: string | null; postCode: string | null; sourceUrl: string | null
}
export type RegionPayload = Omit<RegionRecord, "id">
export const regionsApi = {
  versions: () => api.get<RegionVersion[]>("/api/regions/versions"),
  createVersion: (body: {version: string; label: string; sourceVersion?: string}) => api.post("/api/regions/versions", body),
  list: (params: {version: string; parentCode?: string; search?: string; page?: number; size?: number}) =>
    api.get<{records: RegionRecord[]; total: number; page: number; size: number}>(`/api/regions${qs(params)}`),
  create: (body: RegionPayload) => api.post("/api/regions", body),
  update: (id: string, body: RegionPayload) => api.put(`/api/regions/${encodeURIComponent(id)}`, body),
  remove: (version: string, id: string) => api.del(`/api/regions/${encodeURIComponent(id)}${qs({version})}`),
}
