import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { parseRustResult, validateOwnedPostgres, postgresSuites, runPostgres } from './postgres.mjs'

test('PostgreSQL gate refuses ambient or unrelated database configuration', () => {
  assert.throws(() => validateOwnedPostgres({ DATABASE_URL: 'postgresql://private/db' }))
  assert.throws(() => validateOwnedPostgres({ CI_TEST_POSTGRES_OWNER: 'ci-12345678', TEST_DATABASE_URL: 'postgresql://ci:synthetic@production:5432/postgres' }))
  assert.deepEqual(validateOwnedPostgres({ CI_TEST_POSTGRES_OWNER: 'ci-12345678', TEST_DATABASE_URL: 'postgresql://ci:synthetic@ci-12345678-postgres:5432/postgres' }).database, 'postgres')
})

test('required PostgreSQL result rejects zero tests, ignored tests and mismatched selection', () => {
  assert.deepEqual(parseRustResult('test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 80 filtered out', 8), { passed: 8, failed: 0, skipped: 0 })
  for (const result of ['test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 88 filtered out', 'test result: ok. 0 passed; 0 failed; 8 ignored; 0 measured; 80 filtered out', 'test result: FAILED. 7 passed; 1 failed; 0 ignored; 0 measured; 80 filtered out', '']) assert.throws(() => parseRustResult(result, 8))
  assert.equal(postgresSuites.reduce((count, suite) => count + suite.expected, 0), 76)
})

test('failed and ignored PostgreSQL assertions retain their actual test counts while failing the gate', async () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'ci-pg-counts-'))
  const env = { CI_TEST_POSTGRES_OWNER: 'ci-12345678', TEST_DATABASE_URL: 'postgresql://ci:synthetic@ci-12345678-postgres:5432/postgres' }
  const otherPasses = postgresSuites.slice(1).reduce((count, suite) => count + suite.expected, 0)
  try {
    for (const [name, summary, expected] of [
      ['failed', 'test result: FAILED. 7 passed; 1 failed; 0 ignored; 0 measured; 81 filtered out', { passed: otherPasses + 7, failed: 1, skipped: 0 }],
      ['ignored', 'test result: ok. 0 passed; 0 failed; 8 ignored; 0 measured; 81 filtered out', { passed: otherPasses, failed: 0, skipped: 8 }],
      ['zero', 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 89 filtered out', { passed: otherPasses, failed: 0, skipped: 0 }],
      ['missing', 'cargo: compilation failed before test execution', { passed: otherPasses, failed: 0, skipped: 0 }],
    ]) {
      const outDir = path.join(directory, name)
      const runCargo = async args => {
        const suite = postgresSuites.find(item => args.includes(item.name))
        return { code: suite.name === postgresSuites[0].name ? 1 : 0, timedOut: false, output: suite.name === postgresSuites[0].name ? summary : `test result: ok. ${suite.expected} passed; 0 failed; 0 ignored; 0 measured; 1 filtered out` }
      }
      await assert.rejects(runPostgres({ outDir, env, runCargo }), /PostgreSQL gate failed/)
      const report = JSON.parse(readFileSync(path.join(outDir, 'postgres-result.json'), 'utf8'))
      assert.equal(report.status, 'FAIL'); assert.equal(report.failedSuites, 1)
      assert.deepEqual({ passed: report.passed, failed: report.failed, skipped: report.skipped }, expected)
      if (name === 'missing') assert.equal(report.suites[0].resultMissing, true)
    }
  } finally { rmSync(directory, { recursive: true, force: true }) }
})
