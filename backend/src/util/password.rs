use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

/// Every production password write shares the first-bootstrap requirements.
/// Development keeps the original six-byte compatibility rule.
pub fn validate_for_config(
    password: &str,
    config: &crate::config::Config,
) -> Result<(), crate::error::AppError> {
    if config.is_production() {
        if password.chars().count() < 12
            || password.len() > 1024
            || password.trim().is_empty()
            || password.chars().any(char::is_control)
            || crate::seed::is_demo_password(password)
        {
            return Err(crate::error::AppError::bad_request(
                "生产密码至少 12 个字符，不能使用演示密码、控制字符或空白值",
            ));
        }
    } else if password.len() < 6 {
        return Err(crate::error::AppError::bad_request("密码长度不能少于 6 位"));
    }
    Ok(())
}

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
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_writes_reject_demo_and_short_passwords_with_development_compatibility() {
        let mut config =
            crate::config::Config::from_values(&std::collections::HashMap::new()).unwrap();
        assert!(validate_for_config("admin123", &config).is_ok());
        assert!(validate_for_config("123456", &config).is_ok());
        assert!(validate_for_config("short", &config).is_err());
        config.app_env = "production".into();
        for candidate in [
            "admin123",
            "Zs@12345",
            "Ls@12345",
            "Ww@12345",
            "Tq@12345",
            "Cj@12345",
            "12345678901",
            "            ",
            "long-password\n-invalid",
        ] {
            assert!(validate_for_config(candidate, &config).is_err());
        }
        assert!(validate_for_config(&"x".repeat(1025), &config).is_err());
        assert!(validate_for_config("Synthetic-new-user-password", &config).is_ok());
    }
}
