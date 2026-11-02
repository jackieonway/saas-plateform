/** Guard a real rejection request with the same original carrier deadline.
 * Importing starts nothing. Production callers supply the original deadline
 * and, when needed, an observation of the authority's real current time.
 * Pure tests may inject observations; this helper never changes any clock.
 */

function fail(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
}

async function observe(now) {
  let value;
  try { value = await now(); }
  catch { fail('PHASE23_OLD_CARRIER_WINDOW_INVALID'); }
  if (!Number.isFinite(value)) fail('PHASE23_OLD_CARRIER_WINDOW_INVALID');
  return value;
}

export async function whileUnexpired(options) {
  if (options === null || typeof options !== 'object' || Array.isArray(options)) {
    fail('PHASE23_OLD_CARRIER_WINDOW_INVALID');
  }
  const { expiresAtMs, now = Date.now, run } = options;
  if (!Number.isFinite(expiresAtMs) || typeof now !== 'function' || typeof run !== 'function') {
    fail('PHASE23_OLD_CARRIER_WINDOW_INVALID');
  }
  const before = await observe(now);
  if (expiresAtMs <= before) fail('PHASE23_OLD_CARRIER_EXPIRED_BEFORE_REQUEST');

  // Keep request failures intact. A failed transport is never evidence of
  // revocation, and its caller remains responsible for safe diagnostics.
  const result = await run();
  const after = await observe(now);
  if (after < before) fail('PHASE23_CARRIER_CLOCK_ROLLED_BACK');
  if (expiresAtMs <= after) fail('PHASE23_OLD_CARRIER_EXPIRED_DURING_REQUEST');
  return { result, remainingMs: expiresAtMs - after };
}
