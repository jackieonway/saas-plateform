/** This memory-only key contains credentials. Never log it or include it in telemetry. Unknown fields refuse reuse. */
export function authorizationContextSignature(props: unknown): string | null
/** Does not replace the caller's independent signal identity/aborted/expiry checks. */
export function sameAuthorizationContext(previous: unknown, next: unknown): boolean
