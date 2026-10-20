//! Durable, conservative application-wide protection across remote preflight.
use crate::{error::AppError, state::AppState};
use std::future::Future;
use uuid::Uuid;

pub(crate) async fn track<T>(
    state: &AppState,
    app: Uuid,
    kind: &str,
    work: impl Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
    let id:Uuid=sqlx::query_scalar("INSERT INTO application_release_operations(application_id,kind) VALUES($1,$2) RETURNING id")
        .bind(app).bind(kind).fetch_one(&state.pool).await?;
    // No connection/row lock is held while the work waits on external HTTP.
    // Cancellation or process death leaves a pending record: no TTL guesses.
    let result = work.await;
    // Transport/availability failures do not prove that a COMMIT or remote work
    // has settled. Known SQL/audit rejections can release their completed lease.
    let uncertain = match &result {
        Err(AppError::ReleaseResultUnknown(_) | AppError::Unavailable) => true,
        Err(AppError::Server(error)) => error.downcast_ref::<sqlx::Error>().is_some_and(|error| {
            matches!(
                error,
                sqlx::Error::Io(_)
                    | sqlx::Error::Tls(_)
                    | sqlx::Error::Protocol(_)
                    | sqlx::Error::WorkerCrashed
                    | sqlx::Error::PoolClosed
                    | sqlx::Error::PoolTimedOut
            )
        }),
        _ => false,
    };
    if uncertain {
        return result;
    }
    // This is an operation lease, not history. Removing precisely this completed
    // lease preserves existing audit-failure rollback snapshots. Abandoned leases
    // remain durable; no timeout or retention worker is allowed to remove them.
    let removed = sqlx::query("DELETE FROM application_release_operations WHERE id=$1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if removed.rows_affected() != 1 {
        return Err(AppError::Unavailable);
    }
    result
}
