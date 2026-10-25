//! Transaction hooks for credential generations owned by platform policies.
//! Live readers compare the stored generation even if physical cleanup is delayed.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;

pub(crate) async fn revoke_application_policy_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    application_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query("SELECT public.revoke_application_policy_credentials($1)")
        .bind(application_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(crate) async fn revoke_client_policy_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    client_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query("SELECT public.revoke_client_policy_credentials($1)")
        .bind(client_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
