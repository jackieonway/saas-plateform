import type { MicroEsmModule } from "@/components/micro/types"

// Vite includes these first-party modules in the host build as lazy chunks.
const modules = import.meta.glob<MicroEsmModule>("./*.tsx")

export function hostEsmEntryForCode(code: string): string | null {
  if (!/^[a-z0-9-]+$/.test(code)) return null
  return modules[`./${code}.tsx`] ? `/src/micro-apps/${code}.tsx` : null
}

export function loadHostEsm(entryUrl: string) {
  const path = entryUrl.replace(/^\/src\/micro-apps\//, "./")
  const loader = modules[path]
  if (!entryUrl.startsWith("/src/micro-apps/") || !loader) {
    throw new Error("ESM 模块未包含在当前宿主构建中")
  }
  return loader()
}
