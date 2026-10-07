import { createCipheriv, createDecipheriv, createHash, randomBytes } from 'node:crypto'
import { createReadStream, createWriteStream } from 'node:fs'
import { open, appendFile, lstat, realpath } from 'node:fs/promises'
import { Transform } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import path from 'node:path'

const MAGIC = Buffer.from('SAASPG01')
const TAG_BYTES = 16
const MAX_HEADER_BYTES = 16384
export class RecoveryError extends Error {
  constructor(code) { super(code); this.code = code }
}
export const fail = (code) => { throw new RecoveryError(code) }

export async function safeFile(file, privateMode = true) {
  const info = await lstat(file).catch(() => fail('FILE_UNAVAILABLE'))
  if (!info.isFile() || info.isSymbolicLink()) fail('UNSAFE_FILE')
  if (privateMode && (typeof process.getuid !== 'function' || info.uid !== process.getuid() || (info.mode & 0o077))) fail('UNSAFE_FILE_PERMISSIONS')
  return info
}

export async function containedFile(directory, name) {
  if (!/^[a-z0-9-]+\.(?:enc)$/.test(name)) fail('INVALID_ARTIFACT_NAME')
  const file = path.join(directory, name)
  await safeFile(file)
  if (path.dirname(await realpath(file)) !== await realpath(directory)) fail('ARTIFACT_OUTSIDE_SET')
  return file
}

export function digestStream() {
  const hash = createHash('sha256')
  let bytes = 0
  const stream = new Transform({ transform(chunk, _encoding, callback) { bytes += chunk.length; hash.update(chunk); callback(null, chunk) } })
  return { stream, finish: () => ({ sha256: hash.digest('hex'), bytes }) }
}

export async function encryptStream(input, destination, key, metadata) {
  const iv = randomBytes(12)
  const header = Buffer.from(JSON.stringify({ format: 1, algorithm: 'AES-256-GCM', ...metadata, iv: iv.toString('hex') }))
  if (header.length > MAX_HEADER_BYTES) fail('INVALID_METADATA')
  const prefix = Buffer.alloc(MAGIC.length + 4)
  MAGIC.copy(prefix)
  prefix.writeUInt32BE(header.length, MAGIC.length)
  const aad = Buffer.concat([prefix, header])
  const handle = await open(destination, 'wx', 0o600)
  try {await handle.writeFile(aad)} finally {await handle.close()}
  const cipher = createCipheriv('aes-256-gcm', key, iv)
  cipher.setAAD(aad)
  const plaintext = digestStream()
  await pipeline(input, plaintext.stream, cipher, createWriteStream(destination, { flags: 'a', mode: 0o600 }))
  await appendFile(destination, cipher.getAuthTag())
  return plaintext.finish()
}

export async function decryptStream(source, output, key, expected) {
  const info = await safeFile(source)
  const handle = await open(source, 'r')
  try {
    const prefix = Buffer.alloc(MAGIC.length + 4)
    if ((await handle.read(prefix, 0, prefix.length, 0)).bytesRead !== prefix.length || !prefix.subarray(0, MAGIC.length).equals(MAGIC)) fail('INVALID_ENVELOPE')
    const length = prefix.readUInt32BE(MAGIC.length)
    if (!length || length > MAX_HEADER_BYTES || info.size <= prefix.length + length + TAG_BYTES) fail('INVALID_ENVELOPE')
    const headerBytes = Buffer.alloc(length)
    await handle.read(headerBytes, 0, length, prefix.length)
    let header
    try { header = JSON.parse(headerBytes.toString('utf8')) } catch { fail('INVALID_ENVELOPE') }
    if (header.format !== 1 || header.algorithm !== 'AES-256-GCM' || !/^[0-9a-f]{24}$/.test(header.iv || '')) fail('INVALID_ENVELOPE')
    for (const [keyName, value] of Object.entries(expected || {})) if (header[keyName] !== value) fail('ARTIFACT_METADATA_MISMATCH')
    const tag = Buffer.alloc(TAG_BYTES)
    await handle.read(tag, 0, TAG_BYTES, info.size - TAG_BYTES)
    const decipher = createDecipheriv('aes-256-gcm', key, Buffer.from(header.iv, 'hex'))
    decipher.setAAD(Buffer.concat([prefix, headerBytes]))
    decipher.setAuthTag(tag)
    const plaintext = digestStream()
    try {
      await pipeline(createReadStream(source, { start: prefix.length + length, end: info.size - TAG_BYTES - 1 }), decipher, plaintext.stream, output)
    } catch { fail('ARTIFACT_AUTHENTICATION_FAILED') }
    return { header, ...plaintext.finish() }
  } finally { await handle.close() }
}
