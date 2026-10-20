import { supervise } from '../../scripts/dev.mjs'
import path from 'node:path'

const controller = new AbortController()
process.on('message', () => controller.abort())
const result = await supervise([{
  name: 'parent-exit-fixture', command: process.execPath,
  args: [path.resolve('tests/fixtures/dev-service.mjs'), 'descendant'],
  cwd: process.cwd(), host: '127.0.0.1', port: Number(process.env.FIXTURE_PORT), readyPath: '/',
}], {
  timeoutMs: 4000, signal: controller.signal,
  onReady: (pids) => process.send({ type: 'ready', pids }),
})
process.exit(result)
