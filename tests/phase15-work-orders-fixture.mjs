/** Existing Node work-order service, unchanged, against its own temporary DB. */
import { randomUUID } from 'node:crypto'
import { loadConfig } from '../business-apps/work-orders/backend/config.mjs'
import { createPool, migrate } from '../business-apps/work-orders/backend/db.mjs'
import { createServer } from '../business-apps/work-orders/backend/server.mjs'

export async function startOwnedWorkOrders(fixture, control) {
  // Secrets stay in this Node control plane; they never enter browser init
  // scripts, Vite's environment, screenshots or result documents.
  const config = loadConfig({ NODE_ENV: 'development', WORK_ORDERS_DATABASE_URL: control.databaseUrl,
    PLATFORM_API_ORIGIN: fixture.apiOrigin, PLATFORM_CLIENT_ID: control.clientId,
    PLATFORM_CLIENT_SECRET: control.clientSecret, WORK_ORDERS_HOST: '127.0.0.1',
    WORK_ORDERS_PORT: '0', WORK_ORDERS_ALLOWED_ORIGINS: 'null' })
  const pool = createPool(config)
  pool.on('error', () => {})
  let server
  const ids = { own: randomUUID(), peer: randomUUID(), outside: randomUUID() }
  try {
    await migrate(pool)
    const records = [
      [ids.own, 'PH15-OWN', fixture.target.organizationId, fixture.target.userId, 'Business owner'],
      [ids.peer, 'PH15-PEER', fixture.target.organizationId, fixture.alternateTarget.userId, 'Peer owner'],
      // Same owner as the SELF record, but in a sibling outside the hard cap.
      [ids.outside, 'PH15-OUTSIDE', fixture.outsideTarget.organizationId, fixture.target.userId, 'Business owner'],
    ]
    for (const [id, number, organization, owner, ownerName] of records) {
      await pool.query(`INSERT INTO work_orders(id,number,tenant_id,organization_id,organization_name,
        owner_user_id,owner_display_name,title) VALUES($1,$2,$3,$4,'Owned business fixture',$5,$6,$2)`,
      [id, number, fixture.target.tenantId, organization, owner, ownerName])
    }
    // Use the real authorizer, introspection/context HTTP and SQL scope builder.
    server = createServer({ config, pool, logger: { info() {}, warn() {}, error() {} } })
    await new Promise((resolve, reject) => {
      server.once('error', reject)
      server.listen(0, '127.0.0.1', resolve)
    })
    const origin = 'http://127.0.0.1:' + server.address().port
    return { origin, ids, async close() {
      server.closeAllConnections()
      await new Promise(resolve => server.close(resolve))
      await pool.end()
    } }
  } catch {
    server?.closeAllConnections()
    if (server?.listening) await new Promise(resolve => server.close(resolve))
    await pool.end().catch(() => {})
    throw new Error('Owned existing work-order service fixture failed to start')
  }
}
