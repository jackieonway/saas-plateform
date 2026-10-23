import type { MicroProps } from "./types"

export interface MicroCsvDownload {
  resource: string
  filename: string
  content: string
}

/** Host capabilities are separate from the strict, versioned MicroProps DTO. */
export interface MicroHostServices {
  downloadCsv(request: MicroCsvDownload): void
  dispose(): void
}

const MAX_CSV_BYTES = 4 * 1024 * 1024
const denied = () => new Error("CSV_DOWNLOAD_DENIED")

/** Match the iframe download policy for trusted applications as well. */
export function createMicroServices(
  getProps: () => MicroProps | null,
  options: { saveCsv?: (filename: string, content: string) => void; now?: () => number } = {},
): MicroHostServices {
  let disposed = false
  let lastDownloadAt = Number.NEGATIVE_INFINITY
  const downloads = new Map<string, ReturnType<typeof setTimeout>>()
  const now = options.now ?? (() => performance.now())

  const dispose = () => {
    disposed = true
    for (const [url, timer] of downloads) {
      clearTimeout(timer)
      URL.revokeObjectURL(url)
    }
    downloads.clear()
  }

  const downloadCsv = (request: MicroCsvDownload) => {
    const props = getProps()
    const timestamp = now()
    if (disposed || !props || props.signal.aborted ||
        !(Date.parse(props.expiresAt) > Date.now()) || !(Date.parse(props.tokenExpiresAt) > Date.now()) ||
        !request || typeof request !== "object") throw denied()
    const { resource, filename, content } = request
    if (typeof resource !== "string" || !resource || resource.length > 100 ||
        typeof filename !== "string" || !filename || filename.length > 128 || !/\.csv$/i.test(filename) ||
        /[\\/:*?"<>|]/.test(filename) || [...filename].some(char => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127) || filename.startsWith(".") ||
        typeof content !== "string" || !content || content.length > MAX_CSV_BYTES ||
        new TextEncoder().encode(content).byteLength > MAX_CSV_BYTES ||
        !props.permissions.includes(`${resource}:export`) || !props.dataGrants.some((grant) =>
          grant.resource === resource && grant.action === "export" && grant.subjectUserId === props.user.id &&
          grant.tenantId === props.tenant.id && ["SELF", "ORG", "ORG_SUBTREE", "TENANT"].includes(grant.scope)) ||
        timestamp - lastDownloadAt < 1000) throw denied()

    try {
      if (options.saveCsv) options.saveCsv(filename, content)
      else {
        const url = URL.createObjectURL(new Blob([content], { type: "text/csv;charset=utf-8" }))
        const timer = setTimeout(() => { URL.revokeObjectURL(url); downloads.delete(url) }, 30_000)
        downloads.set(url, timer)
        const link = document.createElement("a")
        link.href = url
        link.download = filename
        link.style.display = "none"
        document.body.appendChild(link)
        try { link.click() } finally { link.remove() }
      }
      lastDownloadAt = timestamp
    } catch { throw denied() }
  }

  return Object.freeze({ downloadCsv, dispose })
}
