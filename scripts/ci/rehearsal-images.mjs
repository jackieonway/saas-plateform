const approvedPostgres = 'postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3'

/** Resolve operator-provisioned images once, then start every container by the
 * inspected immutable identity. A developer's local config ID is not portable. */
export async function resolveRehearsalImages(docker, options = {}, env = process.env) {
  const references = {
    tools: options.toolchainImage || env.CI_TOOLCHAIN_IMAGE || 'saas-ci:phase04',
    postgres: options.postgresImage || env.CI_POSTGRES_IMAGE || approvedPostgres,
  }
  const evidence = {}
  for (const [name, requested] of Object.entries(references)) {
    if (typeof requested !== 'string' || requested.length > 255 || !/^[a-z0-9][a-z0-9./:@_-]*$/.test(requested)
      || (requested.includes('@') && !/^[^@]+@sha256:[a-f0-9]{64}$/.test(requested))) {
      throw new Error('Rehearsal image must be a literal Docker image reference')
    }
    const states = JSON.parse(await docker(['image', 'inspect', requested]))
    const state = Array.isArray(states) && states.length === 1 ? states[0] : null
    if (!/^sha256:[a-f0-9]{64}$/.test(state?.Id || '') || state.Os !== 'linux' || state.Architecture !== 'amd64') {
      throw new Error('Rehearsal images must resolve to one Linux amd64 immutable identity')
    }
    evidence[name] = { requested, id: state.Id, repoDigests: state.RepoDigests || [], platform: 'linux/amd64' }
  }
  return { toolsImage: evidence.tools.id, postgresImage: evidence.postgres.id, evidence }
}
