import { parseEnv } from 'node:util'
import { readFile, realpath, lstat } from 'node:fs/promises'
import path from 'node:path'
import { safeFile, fail } from './envelope.mjs'

const KEYS = new Set(['BACKUP_PLATFORM_DATABASE_URL','BACKUP_WORK_ORDERS_DATABASE_URL','BACKUP_ROOT','BACKUP_KEY_FILE','PG_DUMP_BIN','PG_RESTORE_BIN','PSQL_BIN','PG_CLIENT_LIBRARY_PATH','BACKUP_WRITES_QUIESCED','RESTORE_PLATFORM_MAINTENANCE_URL','RESTORE_WORK_ORDERS_MAINTENANCE_URL','RESTORE_PLATFORM_DATABASE','RESTORE_WORK_ORDERS_DATABASE'])
export function parseDatabaseUrl(value) {
  let url
  try { url = new URL(value) } catch { fail('INVALID_DATABASE_CONFIGURATION') }
  if (!['postgres:', 'postgresql:'].includes(url.protocol) || !url.hostname || !url.username || !url.pathname || url.pathname === '/' || url.hash) fail('INVALID_DATABASE_CONFIGURATION')
  const allowed = new Set(['sslmode','sslrootcert','sslcert','sslkey','connect_timeout'])
  for (const key of url.searchParams.keys()) if (!allowed.has(key)) fail('UNSUPPORTED_DATABASE_OPTION')
  let database, user, password
  try { database = decodeURIComponent(url.pathname.slice(1)); user = decodeURIComponent(url.username); password = decodeURIComponent(url.password) } catch { fail('INVALID_DATABASE_CONFIGURATION') }
  if (!database || database.includes('/') || [database,user,password,url.hostname].some(v => v.includes('\0') || v.includes('\n') || v.includes('\r'))) fail('INVALID_DATABASE_CONFIGURATION')
  const sslmode = url.searchParams.get('sslmode') || 'prefer'
  if (!['disable','allow','prefer','require','verify-ca','verify-full'].includes(sslmode)) fail('INVALID_DATABASE_CONFIGURATION')
  const port = Number(url.port || 5432)
  if (!Number.isInteger(port) || port < 1 || port > 65535) fail('INVALID_DATABASE_CONFIGURATION')
  return { host: url.hostname.replace(/^\[|\]$/g,''), port: String(port), user, password, database, sslmode, sslrootcert: url.searchParams.get('sslrootcert'), sslcert: url.searchParams.get('sslcert'), sslkey: url.searchParams.get('sslkey'), connectTimeout: url.searchParams.get('connect_timeout') || '10' }
}
export const sameDatabase = (a,b) => a.host === b.host && a.port === b.port && a.database === b.database
export function targetName(name, maintenance) {
  if (!/^[a-z][a-z0-9_]{0,62}$/.test(name || '') || ['postgres','template0','template1',maintenance].includes(name)) fail('INVALID_RESTORE_TARGET')
  return name
}
export async function safeDirectory(directory) {
  const info = await lstat(directory).catch(() => fail('DIRECTORY_UNAVAILABLE'))
  if (!info.isDirectory() || info.isSymbolicLink() || info.uid !== process.getuid() || (info.mode & 0o077)) fail('UNSAFE_DIRECTORY_PERMISSIONS')
  return realpath(directory)
}
export async function loadConfig(envFile, mode, backupSet) {
  if (process.platform !== 'linux' || Number(process.versions.node.split('.')[0]) !== 24) fail('LINUX_NODE24_REQUIRED')
  if (!path.isAbsolute(envFile || '')) fail('ABSOLUTE_ENV_FILE_REQUIRED')
  await safeFile(envFile)
  let values
  try { values = parseEnv(await readFile(envFile, 'utf8')) } catch { fail('INVALID_ENV_FILE') }
  for (const key of Object.keys(values)) if (!KEYS.has(key)) fail('UNKNOWN_CONFIGURATION_KEY')
  const keyFile = values.BACKUP_KEY_FILE
  if (!path.isAbsolute(keyFile || '')) fail('ABSOLUTE_KEY_FILE_REQUIRED')
  await safeFile(keyFile)
  const key = await readFile(keyFile)
  if (key.length !== 32) fail('KEY_MUST_BE_32_BYTES')
  for (const name of ['PG_DUMP_BIN','PG_RESTORE_BIN','PSQL_BIN']) {
    if (!path.isAbsolute(values[name] || '')) fail('ABSOLUTE_TOOL_PATH_REQUIRED')
    await safeFile(values[name], false)
  }
  if (values.PG_CLIENT_LIBRARY_PATH && !path.isAbsolute(values.PG_CLIENT_LIBRARY_PATH)) fail('ABSOLUTE_LIBRARY_PATH_REQUIRED')
  const config = { values, key, envFile, mode }
  if (mode === 'backup') {
    if (values.BACKUP_WRITES_QUIESCED !== 'true') fail('WRITES_MUST_BE_QUIESCED')
    if (!path.isAbsolute(values.BACKUP_ROOT || '')) fail('ABSOLUTE_BACKUP_ROOT_REQUIRED')
    config.sources = { platform: parseDatabaseUrl(values.BACKUP_PLATFORM_DATABASE_URL), 'work-orders': parseDatabaseUrl(values.BACKUP_WORK_ORDERS_DATABASE_URL) }
    if (sameDatabase(config.sources.platform, config.sources['work-orders'])) fail('DATABASES_MUST_BE_INDEPENDENT')
  }
  if (backupSet) {
    if (!path.isAbsolute(backupSet)) fail('ABSOLUTE_BACKUP_SET_REQUIRED')
    config.backupSet = await safeDirectory(backupSet)
  }
  const directories = [values.BACKUP_ROOT, config.backupSet].filter(Boolean)
  const resolvedKey = await realpath(keyFile)
  for (const directory of directories) {
    const relative = path.relative(path.resolve(directory), resolvedKey)
    if (!relative || (!relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative))) fail('KEY_MUST_BE_OUTSIDE_BACKUPS')
  }
  if (mode === 'restore') {
    config.targets = {}
    for (const role of ['platform','work-orders']) {
      const prefix = role === 'platform' ? 'RESTORE_PLATFORM' : 'RESTORE_WORK_ORDERS'
      const maintenance = parseDatabaseUrl(values[`${prefix}_MAINTENANCE_URL`])
      config.targets[role] = { maintenance, name: targetName(values[`${prefix}_DATABASE`], maintenance.database) }
    }
    const a = config.targets.platform, b = config.targets['work-orders']
    if (sameDatabase({ ...a.maintenance, database: a.name }, { ...b.maintenance, database: b.name })) fail('DATABASES_MUST_BE_INDEPENDENT')
  }
  return config
}
