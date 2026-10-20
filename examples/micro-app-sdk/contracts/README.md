# Public wire contracts

`schema.json` is the schema source. `x-release-contract` and `x-release-limit`
resolve only through the SDK's `release.json`; they are not additional version or
limit registries. The fixed generator emits standard Draft 2020-12 JSON Schema,
TypeScript declarations, and a standalone Rust serde module. The platform's
`backend/src/public_contract.rs` is an exact generated copy.

```sh
node examples/micro-app-sdk/scripts/generate-contracts.mjs --write
node examples/micro-app-sdk/scripts/generate-contracts.mjs --check
cargo test --locked --manifest-path backend/Cargo.toml --bin saas-platform-backend public_contract_tests
```

The SDK's top-level synchronization command also calls the generator API and
checks these outputs along with browser artifacts. Generation uses Node alone;
Rust formatting is an independent check, so the frontend image does not need a
Rust toolchain. Editing an output directly will fail the synchronization check.

The data declarations are generated from the schema's `$defs`. The generator
keeps two explicit, small templates: TypeScript host-owned `AbortSignal` and
lifecycle callbacks, which are not JSON values; and Rust three-state nullable
fields plus bounded parsing/shape helpers. Those helpers are hand-authored logic,
not an assertion that every JSON Schema keyword has been compiled into Rust.
Use the generated schema when full schema validation is needed. Date-time fields
remain wire strings in the dependency-light Rust module. Consumers must parse
and compare their expiry against a trusted clock before using a context.

The Rust module depends only on serde and serde_json. It contains no HTTP server,
platform client, database model, SQL query, token refresh, or shared storage.
Wire types containing an application token deliberately do not implement `Debug`.
Bounded `from_json` methods replace parser failures with the fixed
`PUBLIC_CONTRACT_INVALID` error and do not retain the input or parser error.
Unused generated DTOs are locally permitted; no business lint is disabled.

`organizationScopeIds` has three distinct states. A missing field on an older
context restricts the consumer to the current organization. An explicit `null`
is retained as the authoritative platform-root response. A list must contain
the current organization and respect the registered limit; it is never silently
truncated. Rust's `OptionalNullable` preserves these states during a round trip.
The parser cannot establish who sent `null`: a backend must first verify its
platform transport and the introspection/context identity binding.

Context and props parsing checks wire versions and bounds, not authorization.
Each protected business request must still validate the application token,
audience, user/tenant/organization binding, current expiry, operation permission,
resource/action grant, and record ownership. Tenant and organization constraints
must remain in the business SQL query. This phase does not migrate the existing
JavaScript backend reference or implement the future Rust business service.

`fixtures/public-wire.json` contains fixed non-secret public examples used by the
Rust consumer tests and available to JavaScript schema tests. Tests compile the
standalone generated module separately from the platform's vendored module and
exercise version rejection, exact round trips, missing/null/list ceilings,
required nullable fields, response limits, tagged notification targets, public
versus internal socket context, and APP/SYSTEM publication coherence. They do
not claim real backend authorization or Socket.IO compatibility.
