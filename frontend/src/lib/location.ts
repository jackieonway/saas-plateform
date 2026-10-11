import { z } from "zod"

export const coordinateField = (label: string, min: number, max: number) => z.string().refine(
  (value) => value.trim() === "" || (Number.isFinite(Number(value)) && Number(value) >= min && Number(value) <= max),
  `${label}范围为 ${min}～${max}`
)

export function hasCoordinatePair(values: { longitude?: string; latitude?: string }): boolean {
  return Boolean(values.longitude?.trim()) === Boolean(values.latitude?.trim())
}

export function coordinateOrNull(value?: string): number | null {
  return value?.trim() ? Number(value) : null
}

export function coordinatePoint(longitude?: string, latitude?: string): { longitude: number; latitude: number } | null {
  const lon = coordinateOrNull(longitude)
  const lat = coordinateOrNull(latitude)
  return lon !== null && lat !== null && Number.isFinite(lon) && Number.isFinite(lat)
    && lon >= -180 && lon <= 180 && lat >= -90 && lat <= 90
    ? { longitude: lon, latitude: lat } : null
}

export function locationText(location: { province?: string; city?: string; district?: string; address?: string }): string {
  return [location.province, location.city, location.district, location.address].filter(Boolean).join(" ")
}

export type ResolvedLocation = { province?: string; city?: string; district?: string; address?: string }

/** Nominatim address keys vary by country and administrative level. */
export function resolvedLocation(data: unknown, approximate = false): ResolvedLocation {
  if (!data || typeof data !== "object") return {}
  const result = data as { address?: Record<string, unknown>; display_name?: unknown }
  const address = result.address ?? {}
  const fallback = typeof result.display_name === "string" ? result.display_name.trim() : ""
  const administrativeDistrict = fallback.split(",").map((part) => part.trim())
    .find((part) => /[区县旗]$/.test(part) && !/(社区|街区)$/.test(part))
  const pick = (...keys: string[]) => keys.map((key) => address[key])
    .find((value): value is string => typeof value === "string" && value.trim().length > 0)?.trim()
  const province = pick("state", "province", "region")
  const city = pick("city", "municipality", "town", "county", "village")
  const district = pick("city_district", "district", "county", "borough")
    || administrativeDistrict || pick("suburb")
  const detail = [pick("neighbourhood", "quarter"), pick("road", "pedestrian", "street"),
    pick("house_number"), pick("building", "amenity")].filter(Boolean).join(" ")
  return {
    province,
    city,
    district: district === city ? undefined : district,
    address: approximate ? undefined : (detail || fallback).slice(0, 300) || undefined,
  }
}
