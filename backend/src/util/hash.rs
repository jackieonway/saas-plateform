use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};

/// SHA-256 + hex (用于 token / secret 存储)
pub fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

/// BASE64URL(SHA256(x)) — PKCE S256
pub fn b64url_sha256(input: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(input.as_bytes()))
}

/// 随机令牌 (字母数字)
pub fn random_token(len: usize) -> String {
    const CS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len).map(|_| CS[rng.gen_range(0..CS.len())] as char).collect()
}

/// 设备码用户码: XXXX-XXXX (去除易混字符 0O1I)
pub fn random_user_code() -> String {
    const CS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    let raw: String = (0..8).map(|_| CS[rng.gen_range(0..CS.len())] as char).collect();
    format!("{}-{}", &raw[0..4], &raw[4..8])
}
