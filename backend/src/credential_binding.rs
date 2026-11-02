//! Transaction hooks for credential generations owned by platform policies.
//! Live readers compare the stored generation even if physical cleanup is delayed.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;

pub(crate) async fn revoke_application_policy_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    application_id: Uuid,
) -> Result<(), AppError> {
    // Only platform-owned derived capabilities are retired here. Canonical
    // OAuth grants belong to identity; its versioned projection is separate.
    sqlx::query("UPDATE public.platform_app_capabilities SET revoked=true WHERE application_id=$1 AND NOT revoked")
        .bind(application_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
