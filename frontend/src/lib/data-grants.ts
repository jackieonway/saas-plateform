import { isUuidV7 } from "@/lib/uuid"
import type {
  DataGrantCapability, DataGrantChange, DataGrantKey, DataGrantOptions,
  DataGrantPreview, DataGrantRow, DataGrantScope, DataGrantSnapshot,
  DataGrantStoredRow, DataGrantTarget, DataGrantUpsert,
} from "@/api/data-grants"

export type DataGrantDraft = Record<string, DataGrantScope | null>

const SCOPES: readonly DataGrantScope[] = ["SELF", "ORG", "ORG_SUBTREE", "TENANT"]
const VERSION = /^[0-9a-f]{64}$/
const IDENTIFIER = /^[a-z][a-z0-9_-]{0,99}$/
const TARGET_FIELDS = ["tenantId", "organizationId", "userId", "membershipId", "applicationId"] as const

export class DataGrantValidationError extends Error {
  constructor() {
    super("数据授权内容无效，请重新读取并检查所选范围")
    this.name = "DataGrantValidationError"
  }
}

function invalid(): never { throw new DataGrantValidationError() }
function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : invalid()
}
function text(value: unknown, max = 100): string {
  return typeof value === "string" && value.length > 0 && value.length <= max ? value : invalid()
}
function identifier(value: unknown): string {
  return typeof value === "string" && value.trim() === value && IDENTIFIER.test(value) ? value : invalid()
}
function uuid(value: unknown): string {
  if (typeof value !== "string" || value.length !== 36 || !isUuidV7(value) || value.toLowerCase() === "00000000-0000-0000-0000-000000000000") invalid()
  return value.toLowerCase()
}
function version(value: unknown): string { return typeof value === "string" && value.length === 64 && VERSION.test(value) ? value : invalid() }
function scope(value: unknown): DataGrantScope {
  return SCOPES.includes(value as DataGrantScope) ? value as DataGrantScope : invalid()
}
function list<T>(value: unknown, max: number, parse: (item: unknown) => T): T[] {
  return Array.isArray(value) && value.length <= max ? value.map(parse) : invalid()
}
function unique<T>(items: T[], key: (item: T) => string): T[] {
  const seen = new Set<string>()
  for (const item of items) {
    const id = key(item)
    if (seen.has(id)) invalid()
    seen.add(id)
  }
  return items
}
function integer(value: unknown, minimum: number, maximum = Number.MAX_SAFE_INTEGER): number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= minimum && value <= maximum ? value : invalid()
}
function pairOrder(left: DataGrantKey, right: DataGrantKey): number {
  return left.resource < right.resource ? -1 : left.resource > right.resource ? 1 : left.action < right.action ? -1 : left.action > right.action ? 1 : 0
}

/** Tuple encoding avoids collisions even when displaying historical key names. */
export function grantKey(key: DataGrantKey): string { return JSON.stringify([key.resource, key.action]) }

export function parseDataGrantTarget(value: unknown): DataGrantTarget {
  const input = record(value)
  return {
    tenantId: uuid(input.tenantId), organizationId: uuid(input.organizationId),
    userId: uuid(input.userId), membershipId: uuid(input.membershipId), applicationId: uuid(input.applicationId),
  }
}

export function sameGrantTarget(left: DataGrantTarget, right: DataGrantTarget): boolean {
  return TARGET_FIELDS.every(field => left[field] === right[field])
}

function parseKey(value: unknown): DataGrantKey {
  const input = record(value)
  return { resource: identifier(input.resource), action: identifier(input.action) }
}
function parseUpsert(value: unknown): DataGrantUpsert {
  const input = record(value)
  return { ...parseKey(input), scope: scope(input.scope) }
}
function parseCapability(value: unknown): DataGrantCapability {
  const input = record(value)
  const scopes = unique(list(input.scopes, 4, scope), item => item)
  if (!scopes.length) invalid()
  return { ...parseKey(input), scopes: scopes.sort() }
}
function parseRow(value: unknown, preview = false): DataGrantRow {
  const input = record(value)
  return {
    id: preview && input.id === null ? null : uuid(input.id),
    // Existing, unregistered pairs are display-only. Do not turn their legacy
    // names into editable capability keys or silently drop them from snapshots.
    resource: text(input.resource), action: text(input.action), scope: scope(input.scope),
    effectiveScope: scope(input.effectiveScope),
    organizationIds: unique(list(input.organizationIds, 10_000, uuid), item => item).sort(),
  }
}
function parseStoredRow(value: unknown): DataGrantStoredRow {
  const row = parseRow(value)
  if (row.id === null) invalid()
  return { ...row, id: row.id }
}
function rows<T extends DataGrantRow>(value: unknown, parse: (item: unknown) => T): T[] {
  const result = unique(list(value, 128, parse), grantKey).sort(pairOrder)
  unique(result.filter(item => item.id !== null), item => item.id!)
  return result
}

export function parseDataGrantSnapshot(value: unknown): DataGrantSnapshot {
  const input = record(value)
  return {
    target: parseDataGrantTarget(input.target), version: version(input.version),
    grants: rows(input.grants, parseStoredRow),
    capabilities: unique(list(input.capabilities, 128, parseCapability), grantKey).sort(pairOrder),
  }
}

export function parseDataGrantPreview(value: unknown): DataGrantPreview {
  const input = record(value), changes = record(input.changes)
  return {
    target: parseDataGrantTarget(input.target), version: version(input.version),
    before: rows(input.before, parseStoredRow), after: rows(input.after, item => parseRow(item, true)),
    changes: {
      added: rows(changes.added, item => parseRow(item, true)),
      updated: rows(changes.updated, parseStoredRow), removed: rows(changes.removed, parseStoredRow),
    },
  }
}

export function parseDataGrantOptions(value: unknown): DataGrantOptions {
  const input = record(value)
  const applications = unique(list(input.applications, 64, value => {
    const application = record(value)
    return { id: uuid(application.id), code: text(application.code), name: text(application.name, 300) }
  }), item => item.id)
  const records = unique(list(input.records, 50, value => {
    const option = record(value)
    if (typeof option.canWrite !== "boolean") invalid()
    return {
      target: parseDataGrantTarget(option.target), username: text(option.username, 300),
      displayName: typeof option.displayName === "string" && option.displayName.length <= 300 ? option.displayName : invalid(),
      tenantName: text(option.tenantName, 300), organizationName: text(option.organizationName, 300),
      canWrite: option.canWrite,
    }
  }), item => JSON.stringify(item.target))
  const size = integer(input.size, 1, 50)
  if (records.length > size) invalid()
  return { applications, records, total: integer(input.total, records.length), page: integer(input.page, 1, 1_000_000), size }
}

/** Detached, deterministic request data. Sorting never ranks scope authority. */
export function canonicalGrantChange(value: DataGrantChange, requireChanges = false): DataGrantChange {
  const input = record(value)
  const upsert = list(input.upsert, 128, parseUpsert).sort(pairOrder)
  const remove = list(input.remove, 128, parseKey).sort(pairOrder)
  unique([...upsert, ...remove], grantKey)
  if (upsert.length + remove.length > 128 || (requireChanges && upsert.length + remove.length === 0)) invalid()
  return { target: parseDataGrantTarget(input.target), expectedVersion: version(input.expectedVersion), upsert, remove }
}

function editable(capability: DataGrantCapability, stored: DataGrantStoredRow | undefined): boolean {
  return stored === undefined || capability.scopes.includes(stored.scope)
}

/** No grant selects a scope automatically. Legacy values remain outside drafts. */
export function createGrantDraft(value: DataGrantSnapshot): DataGrantDraft {
  const snapshot = parseDataGrantSnapshot(value)
  const stored = new Map(snapshot.grants.map(row => [grantKey(row), row]))
  const draft: DataGrantDraft = Object.create(null)
  for (const capability of snapshot.capabilities) {
    const key = grantKey(capability), row = stored.get(key)
    if (editable(capability, row)) draft[key] = row?.scope ?? null
  }
  return draft
}

/** Only touched, registered pairs produce a delta; every retained UUID stays server-owned. */
export function buildGrantChange(value: DataGrantSnapshot, draft: Readonly<DataGrantDraft>): DataGrantChange {
  const snapshot = parseDataGrantSnapshot(value)
  const capabilities = new Map(snapshot.capabilities.map(capability => [grantKey(capability), capability]))
  const stored = new Map(snapshot.grants.map(row => [grantKey(row), row]))
  const upsert: DataGrantUpsert[] = [], remove: DataGrantKey[] = []
  for (const [key, selected] of Object.entries(record(draft))) {
    const capability = capabilities.get(key), row = stored.get(key)
    if (!capability || !editable(capability, row)) invalid()
    if (selected === null) {
      if (row) remove.push({ resource: capability.resource, action: capability.action })
    } else {
      const next = scope(selected)
      if (!capability.scopes.includes(next)) invalid()
      if (!row || row.scope !== next) upsert.push({ resource: capability.resource, action: capability.action, scope: next })
    }
  }
  const inserted = upsert.filter(entry => !stored.has(grantKey(entry))).length
  if (snapshot.grants.length + inserted - remove.length > 128) invalid()
  return canonicalGrantChange({ target: snapshot.target, expectedVersion: snapshot.version, upsert, remove })
}

export function changeFingerprint(change: DataGrantChange): string {
  return JSON.stringify(canonicalGrantChange(change))
}

function sameRows(left: DataGrantRow[], right: DataGrantRow[]): boolean {
  return JSON.stringify([...left].sort(pairOrder)) === JSON.stringify([...right].sort(pairOrder))
}

/** Bind a preview to the exact target/version/delta before enabling confirmation. */
export function assertPreviewMatches(value: DataGrantChange, result: DataGrantPreview): void {
  const change = canonicalGrantChange(value), preview = parseDataGrantPreview(result)
  if (!sameGrantTarget(change.target, preview.target) || change.expectedVersion !== preview.version) invalid()
  const before = new Map(preview.before.map(row => [grantKey(row), row]))
  const after = new Map(preview.after.map(row => [grantKey(row), row]))
  const expected = new Map<string, DataGrantRow>(before)
  const added: DataGrantRow[] = [], updated: DataGrantStoredRow[] = [], removed: DataGrantStoredRow[] = []
  for (const entry of change.upsert) {
    const key = grantKey(entry), prior = before.get(key), next = after.get(key)
    if (!next || next.scope !== entry.scope || next.id !== (prior?.id ?? null)) invalid()
    if (!prior) added.push(next)
    else if (prior.scope !== entry.scope) updated.push({ ...next, id: prior.id })
    else if (!sameRows([prior], [next])) invalid()
    expected.set(key, next)
  }
  for (const entry of change.remove) {
    const key = grantKey(entry), prior = before.get(key)
    if (after.has(key)) invalid()
    if (prior) removed.push(prior)
    expected.delete(key)
  }
  if (!sameRows([...expected.values()], [...after.values()])
      || !sameRows(added, preview.changes.added)
      || !sameRows(updated, preview.changes.updated)
      || !sameRows(removed, preview.changes.removed)) invalid()
}
