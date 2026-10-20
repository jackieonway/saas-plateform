# ZIP artifact node agent

This Linux service installs the platform's canonical ZIP output on exactly one configured node. It does not extract archives, fetch URLs, execute commands, manage containers, or delete installed versions. The only shared crate is the data-only `deployment-contract`.

Build with `cargo build --locked --manifest-path tools/artifact-agent/Cargo.toml`. Run `saas-artifact-agent /absolute/private/agent.json` as an unprivileged account. Terminate using SIGTERM. The agent binds only a loopback HTTP socket; the fixed local HTTPS proxy authenticates normal CA/hostname transport. Each RPC also requires the node-specific bearer token. No unauthenticated health or installation endpoint is exposed.

Configuration is closed (unknown fields fail):

```json
{
  "policy": {
    "schemaVersion": 1, "id": "zip-local", "name": "Isolated nodes",
    "publicOrigin": "https://host.example.test:8443", "applications": ["stores"],
    "nodes": [
      {"id": "node1", "origin": "https://node1.example.test:8443"},
      {"id": "node2", "origin": "https://node2.example.test:8443"}
    ]
  },
  "nodeId": "node1", "bind": "127.0.0.1:3009",
  "rootDirectory": "/data/sub-apps", "tokenFile": "/private/node.token"
}
```

Create the local root ahead of time, owned by the service account, mode 0755 or stricter; it must not be writable by group/others. Keep the token outside the served root, mode 0600, 32–512 URL-safe ASCII characters. Every node has its own token and storage. Paths and their ancestors must not be symlinks. Configure matching policy on platform and all nodes; changing topology changes its SHA-256 revision and invalidates old candidate jobs. Secret files are read again on each request, allowing coordinated credential rotation without changing artifact identity.

`POST /agent/install` accepts only a bounded canonical manifest and base64 file set (49 files, 4 MiB/file, 16 MiB decoded, 24 MiB JSON). SHA-256 identity matches the platform ZIP manifest. `POST /agent/verify` hashes the full actual tree, rejects missing/extra files and returns a nonce-bound receipt. Authentication precedes body parsing; requests have a 45-second deadline and two concurrent slots. The root lock serializes installation/verification. A 0700 private stage is synced then published using Linux `RENAME_NOREPLACE`. Existing matching versions are rehashed/reused; mismatching or corrupt versions are never overwritten or repaired automatically.

Restrict node HTTPS listeners to the platform and gateway network. Only the gateway is a public application entry. Deny dot files, the stage and metadata on the static proxy. The node agent and static proxy may share that node's own local root; neither node shares it with the other node or platform. Production filesystem durability, backup, capacity, per-node access control and service supervision require a separate deployment assessment. This phase's acceptance uses disposable local nodes.

There is no remote cleanup API. Retain active, previous and historical chunks. An operator must investigate corrupted versions and coordinate retention rather than deleting or replacing them through a retry.
