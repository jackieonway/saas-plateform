import type { CSSProperties } from "react"

export const DEFAULT_PRIMARY_COLOR = "#008577"

export const THEME_COLOR_PRESETS = [
  { name: "海青", color: DEFAULT_PRIMARY_COLOR, description: "沉稳清爽" },
  { name: "远山蓝", color: "#2563EB", description: "清晰专业" },
  { name: "鸢尾紫", color: "#7C3AED", description: "柔和雅致" },
  { name: "琥珀", color: "#B45309", description: "温暖明快" },
  { name: "玫瑰", color: "#BE185D", description: "鲜明轻盈" },
  { name: "松绿", color: "#15803D", description: "自然舒展" },
] as const

export function isThemeColor(value: string): boolean {
  return /^#[0-9a-f]{6}$/i.test(value.trim())
}

type Rgb = [number, number, number]
type ThemeVariables = CSSProperties & Record<`--${string}`, string>

function rgb(value: string): Rgb {
  return [1, 3, 5].map((offset) => Number.parseInt(value.slice(offset, offset + 2), 16)) as Rgb
}

function hex(value: Rgb): string {
  return "#" + value.map((channel) => Math.round(channel).toString(16).padStart(2, "0")).join("")
}

function mix(first: Rgb, second: Rgb, weight: number): Rgb {
  return first.map((channel, index) => channel * (1 - weight) + second[index] * weight) as Rgb
}

function luminance(color: Rgb): number {
  const linear = color.map((channel) => {
    const value = channel / 255
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  })
  return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
}

/** Keep text and controls readable even for very light or very dark custom colors. */
function contrasting(color: Rgb, background: Rgb, toward: Rgb, selectedOverlay = 0): Rgb {
  for (let step = 0; step <= 100; step += 1) {
    const candidate = mix(color, toward, step / 100)
    const value = luminance(candidate)
    const backdrop = luminance(mix(background, candidate, selectedOverlay))
    if ((Math.max(value, backdrop) + 0.05) / (Math.min(value, backdrop) + 0.05) >= 4.6) return candidate
  }
  return toward
}

/** Global branding changes the color family; light/dark remains the user's preference. */
export function platformThemeVariables(value: string | undefined, dark: boolean): ThemeVariables {
  const source = rgb(isThemeColor(value ?? "") ? value!.trim() : DEFAULT_PRIMARY_COLOR)
  const white: Rgb = [255, 255, 255]
  const black: Rgb = [0, 0, 0]
  const darkText: Rgb = [15, 23, 42]
  const sidebarBase: Rgb = dark ? [13, 19, 26] : [15, 26, 33]
  const sidebar = mix(sidebarBase, source, 0.1)
  const sidebarAccent = mix(sidebar, white, 0.07)
  const sidebarPrimary = contrasting(source, sidebar, white, 0.2)
  const lightPrimary = contrasting(source, white, black)
  const darkPrimary = contrasting(source, darkText, white)
  const primary = dark ? darkPrimary : lightPrimary
  const foreground = dark ? hex(darkText) : "#ffffff"
  return {
    "--primary": hex(primary),
    "--primary-foreground": foreground,
    "--ring": hex(primary),
    "--chart-1": hex(primary),
    "--accent": hex(mix(dark ? darkText : white, source, dark ? 0.25 : 0.08)),
    "--accent-foreground": hex(dark ? darkPrimary : lightPrimary),
    "--sidebar": hex(sidebar),
    "--sidebar-foreground": "#e5edf1",
    "--sidebar-accent": hex(sidebarAccent),
    "--sidebar-accent-foreground": "#f2f6f8",
    "--sidebar-primary": hex(sidebarPrimary),
    "--sidebar-primary-foreground": hex(sidebar),
    "--sidebar-border": hex(mix(sidebar, white, 0.12)),
    "--sidebar-ring": hex(sidebarPrimary),
  }
}
