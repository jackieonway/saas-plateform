# U06 post-recovery read-only audit

The audit runs only after recovery is active, and after source and final services
have stopped and their runtime LOGIN roles have been fenced. It contains no
epoch, migration, service, HTTP or business-write operation.

```text
python3 backend/ops/recovery/scripts/uuid-u06-final-audit.py --check
python3 backend/ops/recovery/scripts/uuid-u06-final-audit.py --run --output /var/lib/saas-uuid-u06-20261024/final-audit-01.json
```

`--check` never creates a database connection. Both modes require local WSL root
and the exact frozen registration at
`/var/lib/saas-uuid-u06-20261024/operator-inputs/registration.json`, SHA-256
`c1b2c7936af2b29bf5b1015adc8fd7c3b7365ac350a64f84a746a7c0f087600f`.
The registration binds all nine database names, DB/owner OIDs, raw/semantic schema
pins and protected runtime identities. Each input is root-owned, single-link
0600 with a root/0700 direct parent. Evidence output must be new under the fixed
U06 data root, with an existing root/0700 parent; it is created 0600 and fsynced.
Existing files and symlink paths are rejected.

Both modes read existing final/recovery prepared, receipt and activation journals
without importing or invoking the epoch helper. Canonical receipt hashes bind
the targets to the frozen registration; activation records bind the four runtime
OIDs. Recovery additionally requires the completed LOGIN-enable record. The
final pre-activation platform before image must match its canonical commitment;
only the external-client identity/type/status/callback projection is retained.
The helper file itself must still match the SHA already pinned by both journals.

The audit calls the existing read-only data engine inspection, preserving its
pg_catalog/UTC canonical session and exact 12/3/4 migration checks. Each group
must cover 54 tables, 147 scalar/domain/array UUID columns and 67 foreign keys.
UUID values never leave PostgreSQL: each column returns non-null, version-7,
RFC-variant and invalid counts. Physical scans use ONLY to avoid inheritance
double counting. Each FK is independently checked with its ordered local/parent
column pairs and NOT EXISTS. MATCH SIMPLE nullable references are exempt;
MATCH FULL partial-null references are counted as invalid. Unsupported or
unvalidated constraints fail closed.

Sequence observations only read catalogs and last_value/is_called. They verify
registered ownership, bounds, increment, cache=1, exhaustion and the next logical
value against retained owned-column extrema. There is no nextval/setval call.
All 54 row counts are retained per group, including release/deployment,
notification/publication, work-order/idempotency/reminder and audit tables.
The one DISABLED external client and its opaque ID/callback must match the
authenticated pre-activation before image in all three groups. A different
uniquely disabled client in every group is rejected. Notification/publish
membership grants and all four external task
tables must be zero.

Fresh independent maintenance-DB admission observations bracket source and final
scans and must remain equal. Those groups require default read-only databases,
NOLOGIN protected roles and zero other backends/prepared transactions. Recovery
is active: its identity, default read-only and NOLOGIN owner are checked, while
normal runtime connections are permitted. Four LOGIN runtimes must match their
activation OIDs, have no privileged flags or membership edges, and have CONNECT
only to the appropriate recovery database across the exact twelve managed
databases. PUBLIC admission and all other LOGIN CONNECT, inherited/SET-owner and
role-admin bypasses are rejected. Each recovery database freshly proves all four
runtime ACLs: business DML excludes metadata, notification can only select
notifications, and the other services have no table rights. Column-level grants,
sequence UPDATE, grant options, DDL, owner access and MAINTAIN are denied. Only
the owning business service has sequence USAGE/SELECT. Each database uses its own read-only
repeatable-read transaction; this is not a cross-database atomic snapshot.
Original v4 three-database read-only/idle state and both old NOLOGIN roles are
checked separately through the maintenance database, without connecting to v4
business databases. The fresh original-v4 check also rejects every other LOGIN
CONNECT, inherited/SET-owner or role-admin bypass. Sequence state remains a
fresh observation outside MVCC even though ownership/ACL/cache constraints are
independently checked.

The root-owned evidence includes private catalog identities and per-column/FK
proofs. stdout contains totals and an evidence digest only. Failures preserve a
new private failure record, partial observations and the private scratch location;
the audit never overwrites a prior result. No task-table observation is presented
as proof of unobserved external effects or external-owner coordination.

Offline tests:

```text
python3 -m unittest discover -s backend/ops/recovery/scripts/tests -p test_uuid_u06_final_audit.py -v
UUID_U06_FINAL_AUDIT_PG_FIXTURE=isolated-offline-v1 python3 -m unittest discover -s backend/ops/recovery/scripts/tests -p test_uuid_u06_final_audit.py -v
```

The twelve-test opt-in suite uses disposable cached PG17 containers, no network,
ports or host database mounts. It verifies recursive domain/array UUID counting,
composite FK null semantics, real orphan detection, owned sequences, real column
and sequence privilege rejection, grant-option/MAINTAIN rejection, and original-v4
CONNECT and inherited-owner bypass rejection. The offline tests bind canonical
journal commitments, reject a shared wrong external client and exercise missing
or changed runtime OIDs, membership, CONNECT-map and PUBLIC/other-LOGIN failures.
Unrelated LOGIN roles follow the data engine admission contract: SUPERUSER,
CREATEROLE, protected-database CONNECT and protected owner/role escalation are
rejected. CREATEDB alone does not grant rights in an existing protected database;
the fixture accepts that flag only when all actual access/escalation paths are
absent. The four active recovery runtimes independently require all five safe
flags, including NOCREATEDB.
It never reads the production registration or opens any live source/final/recovery
connection. The ordinary suite skips this container case unless explicitly armed.
