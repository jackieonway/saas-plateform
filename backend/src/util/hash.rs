use sha2::{Digest, Sha256};

/// SHA-256 + hex (用于 token / secret 存储)
pub fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}
