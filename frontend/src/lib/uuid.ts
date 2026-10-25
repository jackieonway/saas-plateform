import { v7 } from "uuid"

/** RFC 9562 UUID v7. The library sequences same-millisecond calls and holds its
 * local timestamp on clock rollback; this is not a distributed ordering clock. */
export function newUuid(): string {
  return v7()
}

export function isUuidV7(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)
}
