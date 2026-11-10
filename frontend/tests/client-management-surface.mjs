import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { build } from 'esbuild'
import React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
const require = createRequire(import.meta.url)
const { QueryClient, QueryClientProvider } = require('@tanstack/react-query')
const { MemoryRouter } = require('react-router-dom')

const output = await build({
  entryPoints: [fileURLToPath(new URL('../src/pages/system/oauth/ClientsPage.tsx', import.meta.url))],
  bundle: true, write: false, platform: 'node', format: 'cjs', packages: 'external',
  tsconfig: fileURLToPath(new URL('../tsconfig.json', import.meta.url)),
})
const compiled = { exports: {} }
new Function('require', 'module', 'exports', output.outputFiles[0].text)(createRequire(import.meta.url), compiled, compiled.exports)

test('client management exposes client registration without retired administration panels', () => {
  const query = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  try {
    const html = renderToStaticMarkup(React.createElement(QueryClientProvider, { client: query },
      React.createElement(MemoryRouter, {}, React.createElement(compiled.exports.default))))
    assert.match(html, /客户端管理/)
    assert.match(html, /新增客户端/)
    for (const retired of ['合作方接入', '账号邀请', '同意组管理', '用户同意组', '签名密钥管理', '合作方 UUID']) {
      assert.ok(!html.includes(retired), `${retired} must not be exposed`)
    }
  } finally { query.clear() }
})
