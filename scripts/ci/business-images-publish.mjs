#!/usr/bin/env node
import { appendFileSync, existsSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { createSourceManifest } from './provenance.mjs'
import { inspectOciArchive } from './release.mjs'
import { businessOwnerLabel, publishBusinessImages, validateBusinessCandidate } from './business-images.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const numeric = value => typeof value === 'number' ? Number.isSafeInteger(value) && value > 0 : /^[1-9][0-9]{0,18}$/.test(value || '')
const fail = message => { throw new Error(message) }
const json = file => JSON.parse(readFileSync(file, 'utf8'))

export function candidateArtifactName({ sourceRevision, runId, runAttempt }) {
  if (!/^[a-f0-9]{40}$/.test(sourceRevision || '') || !numeric(runId) || !numeric(runAttempt)) fail('Invalid candidate identity')
  return `business-linux-amd64-candidate-${sourceRevision}-${runId}-${runAttempt}`
}

/** Data returned by the same repository's read-only Actions API. A successful
 * arbitrary workflow, fork, branch, attempt or similarly named artifact is not
 * an approved candidate. Both mandatory aggregate and business jobs must pass. */
export function validateCandidateRun({ run, jobs, artifacts, repository, repositoryId, runId, runAttempt }) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository || '') || !numeric(repositoryId) || !numeric(runId) || !numeric(runAttempt)) fail('Invalid candidate repository/run input')
  if (String(run?.id) !== String(runId) || String(run.run_attempt) !== String(runAttempt) || run.repository?.full_name !== repository || run.head_repository?.full_name !== repository || String(run.repository.id) !== String(repositoryId) || String(run.head_repository.id) !== String(repositoryId) || run.path !== '.github/workflows/production-build.yml' || run.head_branch !== 'main' || !['push', 'workflow_dispatch'].includes(run.event) || run.status !== 'completed' || run.conclusion !== 'success' || !/^[a-f0-9]{40}$/.test(run.head_sha || '')) fail('Candidate must be a completed successful main run of the managed workflow in this repository')
  const jobRows = Array.isArray(jobs) ? jobs : jobs?.jobs
  if (!Array.isArray(jobRows)) fail('Candidate job evidence is missing')
  for (const name of ['Required candidate gate', 'Business images, owned registry and versioned HTTPS routes']) {
    const selected = jobRows.filter(job => job.name === name)
    if (selected.length !== 1 || selected[0].status !== 'completed' || selected[0].conclusion !== 'success' || String(selected[0].run_id) !== String(runId) || (selected[0].run_attempt !== undefined && String(selected[0].run_attempt) !== String(runAttempt))) fail('A required exact-attempt candidate job did not pass')
  }
  const name = candidateArtifactName({ sourceRevision: run.head_sha, runId, runAttempt })
  const artifactRows = Array.isArray(artifacts) ? artifacts : artifacts?.artifacts
  const matches = artifactRows?.filter(artifact => artifact.name === name)
  if (matches?.length !== 1 || matches[0].expired !== false || !numeric(matches[0].id) || !(matches[0].size_in_bytes > 0)) fail('One non-expired verified candidate artifact is required')
  const artifact = matches[0]
  if (!artifact.workflow_run || String(artifact.workflow_run.id) !== String(runId) || String(artifact.workflow_run.repository_id) !== String(repositoryId) || String(artifact.workflow_run.head_repository_id) !== String(repositoryId) || artifact.workflow_run.head_sha !== run.head_sha || artifact.workflow_run.head_branch !== 'main') fail('Candidate artifact belongs to another source/run')
  return { schemaVersion: 1, sourceRevision: run.head_sha, runId: String(runId), runAttempt: String(runAttempt), artifactId: String(artifact.id), artifactName: name, repository, repositoryId: String(repositoryId) }
}

export function verifyCandidateArchives(candidateDir, { sourceRevision, sourceRoot = root } = {}) {
  if (!path.isAbsolute(candidateDir || '')) fail('Candidate directory must be absolute')
  const candidate = validateBusinessCandidate(json(path.join(candidateDir, 'candidate-result.json')), { sourceRevision })
  const recorded = json(path.join(candidateDir, 'source-manifest.json'))
  const current = createSourceManifest(sourceRoot, { sourceRevision: sourceRevision || candidate.sourceRevision })
  if (recorded.sourceRevision !== candidate.sourceRevision || recorded.inputsSha256 !== candidate.inputsSha256 || JSON.stringify(recorded.files) !== JSON.stringify(current.files) || current.inputsSha256 !== candidate.inputsSha256) fail('Candidate actual source files differ from the trusted checked-out commit')
  for (const image of candidate.images) {
    const file = path.join(candidateDir, image.archive)
    if (!existsSync(file)) fail('Required verified image archive is missing')
    const verified = inspectOciArchive(file, candidate.targetPlatform)
    if (verified.archiveSha256 !== image.archiveSha256 || verified.ociManifestDigest !== image.ociManifestDigest || verified.ociConfigDigest !== image.ociConfigDigest || verified.labels[businessOwnerLabel] !== candidate.owner || verified.labels['org.opencontainers.image.revision'] !== candidate.sourceRevision || verified.labels['saas.ci.inputs-sha256'] !== candidate.inputsSha256) fail('Verified candidate archive content or provenance changed')
  }
  return candidate
}

async function github(pathname) {
  const token = process.env.GITHUB_TOKEN
  if (!token) fail('Read-only GitHub token is required')
  const response = await fetch(`https://api.github.com${pathname}`, { headers: { Accept: 'application/vnd.github+json', Authorization: `Bearer ${token}`, 'X-GitHub-Api-Version': '2022-11-28' }, signal: AbortSignal.timeout(30_000) })
  if (!response.ok) fail('GitHub candidate verification request failed')
  return response.json()
}

function argumentsFor(argv) {
  const options = {}, allowed = new Set(['--run-id', '--run-attempt', '--out-file', '--candidate-dir', '--source-revision', '--registry', '--repository-prefix', '--out-dir'])
  for (let index = 0; index < argv.length; index++) {
    const key = argv[index]
    if (['--verify-run', '--verify-candidate', '--publish'].includes(key)) {
      if (options.mode) fail('Exactly one operation is required')
      options.mode = key.slice(2); continue
    }
    if (!allowed.has(key) || options[key] !== undefined || !argv[index + 1] || argv[index + 1].startsWith('--')) fail('Unknown, duplicate or missing publication argument')
    options[key] = argv[++index]
  }
  return options
}

async function main(argv) {
  const options = argumentsFor(argv)
  if (options.mode === 'verify-run') {
    const repository = process.env.GITHUB_REPOSITORY, repositoryId = process.env.GITHUB_REPOSITORY_ID, runId = options['--run-id'], runAttempt = options['--run-attempt'], outFile = options['--out-file']
    if (!numeric(runId) || !numeric(runAttempt) || !path.isAbsolute(outFile || '') || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository || '')) fail('Explicit run, attempt, repository and absolute output required')
    const base = `/repos/${repository}/actions/runs/${runId}`
    const [run, jobs, artifacts] = await Promise.all([github(base), github(`${base}/attempts/${runAttempt}/jobs?filter=all&per_page=100`), github(`${base}/artifacts?per_page=100`)])
    if (jobs.total_count > 100 || artifacts.total_count > 100) fail('Candidate metadata exceeds the bounded verification page')
    const verified = validateCandidateRun({ run, jobs, artifacts, repository, repositoryId, runId, runAttempt })
    writeFileSync(outFile, JSON.stringify(verified, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `head_sha=${verified.sourceRevision}\nartifact_id=${verified.artifactId}\nartifact_name=${verified.artifactName}\nrun_id=${verified.runId}\nrun_attempt=${verified.runAttempt}\n`)
    console.log(JSON.stringify({ ok: true, sourceRevision: verified.sourceRevision, artifactName: verified.artifactName })); return
  }
  if (!['verify-candidate', 'publish'].includes(options.mode)) fail('Select --verify-run, --verify-candidate or --publish')
  const candidateDir = options['--candidate-dir'], sourceRevision = options['--source-revision']
  if (!/^[a-f0-9]{40}$/.test(sourceRevision || '')) fail('Trusted source revision is required')
  const candidate = verifyCandidateArchives(candidateDir, { sourceRevision })
  if (options.mode === 'verify-candidate') { console.log(JSON.stringify({ ok: true, sourceRevision: candidate.sourceRevision, images: candidate.images.length })); return }
  if (process.env.BUSINESS_PUBLICATION_ALLOWED !== 'protected-release') fail('Publication is restricted to the explicit protected release job')
  const result = await publishBusinessImages({ candidate, candidateDir, registry: options['--registry'], repositoryPrefix: options['--repository-prefix'],
    scope: 'protected-release', allowProtectedPublication: true, outDir: options['--out-dir'] })
  console.log(JSON.stringify({ status: result.status, registryPublication: 'PASS', productionDeployment: 'NOT_RUN', images: result.images.length }))
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { await main(process.argv.slice(2)) }
  catch (error) { console.error(error.message); process.exitCode = 1 }
}
