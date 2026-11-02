/** Browser transport and owned-browser teardown for the Phase25 harness. */
export async function fetchBusinessJson(input) {
  const response = await fetch(input.endpoint, {
    method: input.method,
    credentials: input.endpoint.startsWith('/') ? 'same-origin' : 'omit',
    cache: 'no-store', redirect: 'error',
    headers: { Accept: 'application/json',
      ...(input.bearer ? { Authorization: `Bearer ${input.bearer}` } : {}),
      ...(input.csrf ? { 'X-CSRF-Token': input.csrf } : {}),
      ...(input.idempotencyKey ? { 'Idempotency-Key': input.idempotencyKey } : {}),
      ...(input.body === undefined ? {} : { 'Content-Type': 'application/json' }) },
    ...(input.body === undefined ? {} : { body: JSON.stringify(input.body) }),
  });
  let body; try { body = await response.json(); } catch { body = null; }
  return { status: response.status, body };
}

// Health intentionally has no browser CORS policy. Navigate to its origin,
// then restore the Console origin before any Socket.IO request is issued.
export async function navigateBusinessReadiness(page, readyUrl, returnUrl) {
  try {
    const response = await page.goto(readyUrl, { waitUntil: 'domcontentloaded' });
    if (response?.status() !== 200 || response.url() !== readyUrl) {
      throw new Error('PHASE25_BUSINESS_NOTIFICATION_READY_FAILED');
    }
  } finally {
    await page.goto(returnUrl, { waitUntil: 'domcontentloaded' });
  }
}

export async function closeBusinessBrowser(browser, contexts) {
  let failures = 0;
  await Promise.all(contexts.map(async context => {
    try { await context.close(); } catch { failures++; }
  }));
  try { await browser?.close(); } catch { failures++; }
  let contextsClosed = false, browserClosed = false;
  try { contextsClosed = contexts.every(context => context.pages().length === 0); } catch { failures++; }
  try { browserClosed = !browser || !browser.isConnected(); } catch { failures++; }
  return { contextsClosed, browserClosed, failures };
}
