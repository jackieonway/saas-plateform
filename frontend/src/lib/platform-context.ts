import type { ProfileResp } from "@/api/types"

/** The server's context capability is authoritative, including an explicit false.
 * Only legacy responses that omit it fall back to the physical tenant flag. */
export function hasPlatformContext(profile: Pick<ProfileResp, "tenant" | "platformContext"> | null | undefined): boolean {
  if (profile?.platformContext !== undefined) return profile.platformContext === true
  return profile?.tenant?.isPlatform === true
}
