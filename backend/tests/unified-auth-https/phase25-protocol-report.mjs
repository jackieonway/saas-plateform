import assert from 'node:assert/strict';
import fs from 'node:fs';

/** Keep the test verdict separate from HTTP facts supplied by a check. */
export function completedProtocolCheck(stage, scenarioIds, facts) {
  assert(facts && typeof facts === 'object' && !Array.isArray(facts), 'PHASE25_PROTOCOL_CHECK_FACTS');
  for (const name of ['stage', 'status', 'scenarioIds']) {
    assert(!Object.hasOwn(facts, name), 'PHASE25_PROTOCOL_CHECK_RESERVED_FIELD');
  }
  return { stage, status: 'PASS', scenarioIds, ...facts };
}

/** ftruncate does not reset an open descriptor's current file position. */
export function rewriteProtocolReport(fd, text) {
  const bytes = Buffer.from(text, 'utf8');
  fs.ftruncateSync(fd, 0);
  let written = 0;
  while (written < bytes.length) {
    const count = fs.writeSync(fd, bytes, written, bytes.length - written, written);
    assert(count > 0, 'PHASE25_PROTOCOL_REPORT_WRITE_STALLED');
    written += count;
  }
  fs.fsyncSync(fd);
}
