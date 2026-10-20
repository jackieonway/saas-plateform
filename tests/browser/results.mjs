/** Classify a mandatory script's emitted check groups without treating skips as success. */
export function classifyBrowserResult({ output, exitCode, expectedCheckGroups, error }) {
  const pass = (output.match(/^PASS(?:\s|:)/gm) || []).length;
  const skip = (output.match(/^(?:SKIP|NOT_RUN)(?:\s|:)/gm) || []).length;
  const reportedFailures = (output.match(/^FAIL(?:\s|:)/gm) || []).length;
  const fail = exitCode !== 0 || error || reportedFailures || pass !== expectedCheckGroups || skip > 0 ? 1 : 0;
  return { pass, fail, skip, reportedFailures };
}
