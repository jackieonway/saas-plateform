//! Opaque nonce generation for private BFF/session contracts.
use crate::error::AppError;
use rand::RngCore;
pub(crate) fn opaque_secret() -> Result<String, AppError> {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| AppError::Unavailable)?;
    Ok(hex::encode(bytes))
}
