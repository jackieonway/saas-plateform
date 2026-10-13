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

export function locationText(location: ResolvedLocation): string {
  return [location.province, location.city, location.district, location.township, location.village, location.address].filter(Boolean).join(" ")
}

export type ResolvedLocation = {
  province?: string; city?: string; district?: string; township?: string; village?: string; address?: string
}

/** Nominatim address keys vary by country and administrative level. */
export function resolvedLocation(data: unknown, approximate = false): ResolvedLocation {
  if (!data || typeof data !== "object") return {}
  const result = data as { address?: Record<string, unknown>; display_name?: unknown }
  const address = result.address && typeof result.address === "object" ? result.address : {}
  const fallback = typeof result.display_name === "string" ? result.display_name.trim() : ""
  const parts = fallback.split(/[,，]/).map((part) => part.trim())
  const isDistrict = (name: string) => /[区县旗]$/.test(name) && !/(社区|街区|自治区|特别行政区)$/.test(name)
  const isMunicipality = (name: string) => /^(北京|天津|上海|重庆)市?$/.test(name)
  const isTownship = (name: string) => /(?:乡|镇|街道|街道办事处|苏木)$/.test(name)
  const isVillage = (name: string) => /(?:村|社区|屯|嘎查|村委会|居委会|村民委员会|居民委员会)$/.test(name)
  const administrativeDistrict = parts.find(isDistrict)
  const pick = (...keys: string[]) => keys.map((key) => address[key])
    .find((value): value is string => typeof value === "string" && value.trim().length > 0)?.trim()
  const municipality = pick("municipality")
  const explicitCity = pick("city") || (municipality && !isTownship(municipality) && !isVillage(municipality) ? municipality : undefined)
  const province = pick("state", "province", "region")
    || parts.find((part) => /(?:省|自治区|特别行政区)$/.test(part) || isMunicipality(part))
    || (explicitCity && isMunicipality(explicitCity) ? explicitCity : undefined)
  const county = pick("county")
  // Nominatim frequently puts Chinese counties under "county"; those are districts,
  // while a prefecture such as an autonomous prefecture belongs at city level.
  // Display names list the address from the most local part to its parents.
  const displayedCities = parts.filter((part) => /(?:市|自治州|地区|盟)$/.test(part)
    && part !== province && !isDistrict(part))
  const displayedCity = displayedCities[displayedCities.length - 1]
  const cityStem = (name: string) => name.replace(/市$/, "")
  const explicitCityIndex = explicitCity ? parts.findIndex((part) => cityStem(part) === cityStem(explicitCity)) : -1
  const countyCity = explicitCity && explicitCityIndex >= 0 && displayedCity
    && parts.lastIndexOf(displayedCity) > explicitCityIndex
    && cityStem(displayedCity) !== cityStem(explicitCity)
    ? explicitCity : undefined
  const prefecture = county && /(?:自治州|地区|盟)$/.test(county) ? county
    : displayedCity && (displayedCity !== county || administrativeDistrict) ? displayedCity : undefined
  const city = (countyCity ? prefecture : explicitCity) || prefecture
    || (province && isMunicipality(province) ? province : undefined)
  const suburb = pick("suburb")
  const district = countyCity || pick("city_district", "district", "borough")
    || (county && county !== city && county !== province ? county : undefined)
    || administrativeDistrict || (suburb && isDistrict(suburb) ? suburb : undefined)
  const township = [pick("town"), pick("township"), pick("subdistrict"), suburb, municipality]
    .find((name): name is string => typeof name === "string" && isTownship(name)) || parts.find(isTownship)
  const village = [pick("village"), pick("neighbourhood"), pick("quarter"), pick("hamlet")]
    .find((name): name is string => typeof name === "string" && isVillage(name)) || parts.find(isVillage)
  const detail = [pick("neighbourhood", "quarter"), pick("road", "pedestrian", "street"),
    pick("house_number"), pick("building", "amenity")].filter(Boolean).join(" ")
  return {
    province,
    city,
    district: district === city ? undefined : district,
    township: approximate ? undefined : township,
    village: approximate ? undefined : village,
    address: approximate ? undefined : (detail || fallback).slice(0, 300) || undefined,
  }
}
