import assert from 'node:assert/strict'
import test from 'node:test'
import { existsSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

test('every declared frontend test entrypoint is shipped in the checkout', () => {
  const root = new URL('../', import.meta.url)
  const { scripts } = JSON.parse(readFileSync(new URL('package.json', root), 'utf8'))
  let checked = 0
  for (const [name, command] of Object.entries(scripts)) {
    if (name !== 'test' && !name.startsWith('test:')) continue
    for (const match of command.matchAll(/(?:^|\s)(tests\/[^\s]+\.mjs)(?=\s|$)/g)) {
      const entrypoint = new URL(match[1], root)
      assert.ok(existsSync(entrypoint), `${name}: missing ${fileURLToPath(entrypoint)}`)
      checked++
    }
  }
  assert.ok(checked >= 20, 'The unit and browser checks must remain declared')
})
