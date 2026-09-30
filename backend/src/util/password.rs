use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

/// Argon2 现算密码哈希
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("密码哈希计算失败: {e}"))?;
    Ok(hash.to_string())
}

/// Argon2 校验 (哈希格式非法时返回 false)
pub fn verify_password(hash: &str, password: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(p) => p,
        Err(_) => return false,
    };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}
