export const qs = values => {
  const query = new URLSearchParams(Object.entries(values).filter(([,value]) => value !== undefined))
  return query.size ? '?' + query : ''
}
async function request(method, path, body) {
  const response = await fetch(new URL(path, process.env.PHASE31_ORIGIN), {
    method, headers: { Authorization: 'Bearer ' + process.env.PHASE31_TOKEN, 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(10000), redirect: 'error',
  })
  if (!response.ok) throw Error('old adapter HTTP status ' + response.status)
  return response.json()
}
export const api = { get: path => request('GET', path), post: (path, body) => request('POST', path, body) }
