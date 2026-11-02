# Phase 01 — Unified contract and callsite inventory

Date: 2026-10-27  
Status: COMPLETED (documentary acceptance only)

## Inputs and baseline

- Read the project ledger, root/parent `AGENTS.md`, workspace `README.md`, unified architecture, adopted policy decisions, and the eight repository READMEs and Cargo/npm manifests.
- Captured `git rev-parse HEAD` and rechecked full `git status --porcelain=v1 -uall` for all eight independent repositories. `saas-identity` already had a modified README; `saas-plateform` already had a modified docs index plus untracked architecture, policy and ledger input documents. The other six were clean. The three files under `unified-auth/` are this phase's new outputs. Exact HEADs and status are in [callsite inventory](../callsite-inventory.md).
- Source searches and targeted reads confirmed both registries, both public issuance paths, current admin/UI proxies, BFFs, business resource consumers, notification proof flow, host/SDK and external example locations.

## Deliverables

- [Implementation contract](../implementation-contract.md): entity dimensions, DTO examples, operation receipts, protocol/error surface, versions, state machines, lock ordering, consent/revocation, resource authorization, roles/step-up and adopted defaults.
- [Callsite inventory](../callsite-inventory.md): repository baseline, current registry/routes/consumers, phase ownership and Q3/Q8/Q9/Q10 release gates.

## Self-check

| Acceptance criterion | Result | Notes |
|---|---|---|
| Every current registry, protocol entry and BFF/business/notification consumer has a source locator and replacement phase | PASS | Inventory lists all eight repositories and assigns replacement responsibilities through Phases 2–24. |
| USER/MACHINE, ACCOUNT_ONLY/PLATFORM_CONTEXT, sector, consent group and online/offline limits remain distinct | PASS | Separate dimensions, flows, state machines and defaults are explicit in the contract. |
| DTO, errors, lock/version/idempotency/unknown-result recovery have examples and contain no real secrets | PASS | Both projection and client DTOs use illustrative UUIDs only; command recovery is bound to original operation ID/digest. |
| Each unresolved item has impact, owner and a later-stage gate | PASS | Q3/Q8/Q9/Q10 gate table names owner roles, missing evidence and phase gates; closed policy questions link to decisions. |

## Verification and resources

- Commands: `git -C <repo> rev-parse HEAD`, `git -C <repo> status --short --branch`, `rg` source inventory and PowerShell source/document reads. No code or schema was changed.
- Verification level: documentary/source inspection only; no unit, PostgreSQL, HTTP, browser or deployment tests are applicable to this Phase 1 scope.
- Docker/containers: none created or reused. No temporary files, databases, credentials or other resources were created. Docker disk usage was not queried because no Docker resource was touched.
- Links: output files link to the architecture and policy documents in the same `designs` directory; repository paths are rooted in the workspace and phase-report links are relative. Existing untracked/modified input files were preserved; the task ledger status/pointer and the existing docs index links were updated for these outputs; other pre-existing changes were preserved.

## Risks / notes

This is a frozen implementation contract, not proof that any target record/API is implemented. Q3/Q8/Q9/Q10 release facts remain open. The lock order is a proposed common serialization contract and must be checked against actual SQL in Phases 7–10 and 20 before implementation is accepted.
