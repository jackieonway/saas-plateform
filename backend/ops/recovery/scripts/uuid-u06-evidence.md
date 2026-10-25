# U06 authenticated evidence bridge

This bridge only seals or opens private files. It does not extract archives,
connect to a database, change UUIDs, or operate services. Existing UUID export,
transform, import and `uuid-seal-evidence` contracts are unchanged.

Use the protected Linux executable `/usr/local/libexec/saas-recovery-u06-codec`.
The caller must pin its SHA-256 in the private U06 configuration.

## Seal

The existing command remains:

```text
saas-recovery-u06-codec uuid-seal-evidence /absolute/private/seal.json
```

The strict seal configuration fields are `profile`, `input`, `output`,
`key_file`, and `purpose`. `profile` remains `uuid-local-preview`. The plaintext
input must be in private tmpfs; the ciphertext output must be a new file.
U06 callers use the same purpose convention as the opening contract below.
Its successful report has status `PRIVATE_EVIDENCE_SEALED`, plaintext `sha256`,
`plaintext_bytes`, and `frames`.

## Open

```text
saas-recovery-u06-codec uuid-open-evidence /absolute/private/open.json
```

The opening configuration rejects duplicate or unknown JSON fields:

```json
{
  "profile": "uuid-local-u06",
  "input": "/var/lib/saas-uuid-u06-20261024/a-source/platform.enc",
  "output": "/dev/shm/saas-uuid-u06-20261024/a-source/platform.tar",
  "key_file": "/var/lib/saas-uuid-u06-keys-20261024/snapshot.key",
  "purpose": "saas-uuid-preview-freeze/1/a-source/u06-platform",
  "expected_sha256": "REPLACE_WITH_THE_64_LOWERCASE_HEX_PLAINTEXT_SHA256"
}
```

`expected_sha256` is the plaintext digest from the sealing report, not a hash of
the ciphertext. The batch is the first directory below both fixed input/output
roots and must exactly equal the purpose's batch segment. Batch and logical
name use at most 100 ASCII letters, digits, underscores or hyphens; the logical
name must begin `u06-` and include a nonempty suffix. The caller must use the
exact original sealing purpose. A different purpose fails authentication.

All directories at or below the input/output roots must already exist, belong
to the invoking user, and have mode 0700. The output root and output parent must
actually reside on tmpfs. Configuration, input and key must be owned mode-0600
regular files with one hard link; the key contains exactly 32 raw bytes. Paths
are absolute and canonical, with no symlink component, dot or empty segment.
The key path may be outside the evidence roots but follows the same private
file checks. The command rejects any existing output, including a symlink.

Opening reads configuration, key and ciphertext through validated file handles.
It decrypts into a private temporary tmpfs file, verifies the complete encrypted
footer and absence of trailing bytes, and compares the expected plaintext
digest. Only then does an atomic no-replace publication create the output.
Success requires staging-link retirement, parent synchronization and a final
single-link mode-0600 inode. Failure removes this invocation's staging data and
any uncommitted publication with the same inode; existing files are preserved.

The successful JSON contains only `status: PRIVATE_U06_EVIDENCE_OPENED`,
`sha256`, `plaintext_bytes`, and `frames`. Errors are static redacted messages.
There is no plaintext-on-disk fallback or overwrite option.
