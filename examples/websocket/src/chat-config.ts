export function parseChatEndpoint(value: string | undefined): string | null {
  const candidate = value?.trim();
  if (!candidate) return null;
  try {
    const url = new URL(candidate);
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password ||
      url.search || url.hash || url.pathname !== '/') {
      throw new Error();
    }
    return url.origin;
  } catch {
    // Do not reflect potentially sensitive invalid input into build output or browser errors.
    throw new Error('VITE_CHAT_ENDPOINT must be a public HTTP(S) origin without credentials, query, fragment or path');
  }
}
