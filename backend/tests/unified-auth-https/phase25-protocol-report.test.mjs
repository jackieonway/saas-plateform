import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { completedProtocolCheck, rewriteProtocolReport } from './phase25-protocol-report.mjs';

test('repeated report persistence replaces bytes at position zero despite a moved descriptor, UTF8 and shrinking output', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'phase25-protocol-report-'));
  const filename = path.join(directory, 'report.json');
  const fd = fs.openSync(filename, 'wx', 0o600);
  try {
    fs.writeSync(fd, 'preexisting descriptor offset');
    for (const value of [{ checks: ['long'.repeat(256)] }, { checks: ['中文', 'é'], result: 'FAIL' }, { result: 'PASS' }]) {
      const expected = JSON.stringify(value, null, 2) + '\n';
      rewriteProtocolReport(fd, expected);
      const actual = fs.readFileSync(filename);
      assert.deepEqual(actual, Buffer.from(expected));
      assert(!actual.includes(0));
      assert.deepEqual(JSON.parse(actual.toString('utf8')), value);
    }
  } finally {
    fs.closeSync(fd);
    fs.unlinkSync(filename);
    fs.rmdirSync(directory);
  }
});

test('HTTP denials retain a PASS verdict and reserved fact fields are rejected', () => {
  assert.deepEqual(completedProtocolCheck('AUTHORIZE_NEGATIVE_PLAIN', ['P25-C05-S02'], { httpStatus: 400, denied: true }), {
    stage: 'AUTHORIZE_NEGATIVE_PLAIN', status: 'PASS', scenarioIds: ['P25-C05-S02'], httpStatus: 400, denied: true,
  });
  for (const facts of [{ status: 400 }, { stage: 'forged' }, { scenarioIds: [] }]) {
    assert.throws(() => completedProtocolCheck('EXPECTED', [], facts), /PHASE25_PROTOCOL_CHECK_RESERVED_FIELD/);
  }
});
