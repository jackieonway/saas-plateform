# U06 local credential epoch handoff

`prepare-uuid-u06-epoch.py` is a separate, standard-library-only local WSL root
helper. It does not build binaries, operate services, issue HTTP requests, seed
data, or alter U05/source roles. Formal migration history remains 12/3/4.

Every input is strict UTF-8 JSON with no duplicate or unknown fields. Files must
be single-link root-owned 0600 regular files with direct root-owned 0700 parents.
Canonical Linux paths cannot contain symlinks, empty components, dot components,
backslashes or control characters. Database operations use only local postgres
peer authentication on `/run/postgresql:5432` with a cleared environment.

The input DTO is exactly:

```json
{
  "format": "saas-uuid-u06-epoch-input/1",
  "generation": "final",
  "target_receipt_file": "/var/lib/saas-uuid-u06-20261024/receipts/final.json",
  "source_env_files": {
    "platform": "/var/lib/saas-uuid-u05-runtime-20261024/env/platform.env",
    "stores": "/var/lib/saas-uuid-u05-runtime-20261024/env/stores.env",
    "work_orders": "/var/lib/saas-uuid-u05-runtime-20261024/env/work_orders.env",
    "notification": "/var/lib/saas-uuid-u05-runtime-20261024/env/notification.env",
    "examples_oauth": "/var/lib/saas-uuid-u05-runtime-20261024/env/examples_oauth.env",
    "examples_chat": "/var/lib/saas-uuid-u05-runtime-20261024/env/examples_chat.env"
  }
}
```

For recovery, generation is `recovery` and all six source env paths are fixed to
`/var/lib/saas-uuid-u06-runtime-20261024/final/env/<service>.env`.
The target receipt has exactly `format="saas-uuid-u06-target-receipts/1"`,
`generation`, and `targets`. Targets has exactly platform/stores/work_orders;
each item has database, database_oid, owner, owner_oid, schema_sha256, import_marker.
Both OIDs are positive integers. All three markers bind the same canonical RFC
UUIDv7 batch and package hash. Names and owners are generation-specific fixed
allowlists; sources and opposite-generation targets are rejected.

Commands are deliberately separate:

1. `--check --config <file>` validates the offline DTO/receipt without PostgreSQL.
2. `--prepare --config <file>` reads the restored targets, their full raw schemas,
   exact migration fingerprints, empty six credential tables, existing clients,
   application bindings and users. It verifies fresh role names are unused and
   every existing managed protected database has no PUBLIC admission. It then
   creates `/var/lib/saas-uuid-u06-20261024/epoch-<generation>` exactly once.
3. `--activate --journal <epoch-dir>` creates four least-privilege roles NOLOGIN,
   rotates all 12 confidential client hashes, and increments every user session
   version once in the platform transaction. Full before/after images permit no
   other client/user field change. The external DISABLED client remains DISABLED.
   Runtime permissions are granted and independently reread per database.
4. `--enable-login --journal <epoch-dir>` is reserved for the operator after T0
   capture and warm recovery. It rechecks exact identities, schemas, ACLs and
   unchanged pre-open platform data before enabling only the four new roles.
5. `--fence-login --journal <epoch-dir>` disables only this generation's four
   roles after the operator stops its services. Live target sessions must already
   be zero. It does not terminate unrelated sessions or operate services.

Every action creates a fresh phase directory and fsyncs intent before mutation.
Database commits are separate. A failed or interrupted activation has no complete
receipt and cannot be repeated; roles remain NOLOGIN. No automatic rollback,
LOGIN restoration, DROP/TRUNCATE or destructive retry exists. The entire group
stays quarantined for operator investigation. Existing phase directories reject
reuse even if a process failed before its first database write.

The new private output contains six `<service>.json` inputs with exactly the U05
input field sets. Only four runtime-consumed confidential secrets enter these
service inputs. The separate `client-secrets.private.json` is
`{format:"saas-uuid-u06-client-secrets/1",generation,clients:{opaque_client_id:raw_secret}}`
for all 12 clients, exclusively for controlled acceptance. Role passwords have
their own private input. Journals and public epoch DTO contain hashes, not raw
client secrets. The private materials are retained; this helper has no cleanup
or overwrite command.

`credential-epoch.json` is exactly the runtime handoff DTO:
format=`saas-uuid-u06-credential-epoch/1`, generation, jwt_secret_sha256,
previous_jwt_secret_sha256, confidential_clients, secrets_fresh=true. Each of 12
client items has client_id/previous_secret_hash/new_secret_hash. Previous JWT is
read from the actual previous generated platform env. Previous client hashes are
read from restored rows and checked against the four consumed old raw inputs.
New JWT, all metrics, all client secrets and four role passwords are independent.

Activation proof remains separate from the runtime generator's imported receipt.
`activate/complete.json` binds four role OIDs, the epoch and full before/after
platform hashes; per-database after records contain actual ACL proof. Managed
admission covers the 12 fixed original-v4, U05, final and recovery database names.
It is not an assertion about unrelated databases on this local cluster.

Offline tests:

```text
python3 -m unittest discover -s backend/ops/recovery/scripts/tests -p test_prepare_uuid_u06_epoch.py -v
UUID_U06_EPOCH_PG_FIXTURE=isolated-offline-v1 python3 -m unittest discover -s backend/ops/recovery/scripts/tests -p test_uuid_u06_epoch_pg.py -v
```

The second test requires an already cached postgres:17.6-bookworm image. It creates
one disposable container with no network, ports or host socket/database mounts,
uses only its full container ID, and removes only that container. It exercises
real SQL, quoted free-form before images, raw schema guards, least privileges,
NOLOGIN→LOGIN→NOLOGIN, and stale-before-image transaction rollback. It does not
access the host PostgreSQL server or any real runtime material.

## Canonical catalog session regression

The first real U06 epoch preflight exposed a search-path-dependent raw schema
digest: PostgreSQL deparsers such as `pg_get_constraintdef` qualify names
differently under `public` and `pg_catalog`. The registration was captured under
`pg_catalog`, while the original epoch runner inherited the default search path.
The failing preflight did not create the epoch directory or runtime roles.

The runner now pins `search_path=pg_catalog` and `timezone=UTC` in PGOPTIONS at
connection startup for both read-only and mutation sessions. The offline runner
regression asserts those options alongside the read-only default and cleared
environment. This supplements transaction-local settings; it does not change the
registered schema or relax a raw digest comparison. The activated helper source
is journal-pinned and must not be edited after preparation.
