//! Bounded request-local snapshot reuse, never a cross-request authorization cache.
use std::future::Future;

/// A database revision is only a change detector, never an authentication credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision(pub [u8; 32]);

/// Consistent data and the revision observed in the same database snapshot.
pub struct Snapshot<T> {
    /// Authorization data assembled by the trusted persistence adapter.
    pub value: T,
    /// Revision read inside that snapshot.
    pub revision: Revision,
}

/// Persistence failures retain their cause; continuous change fails closed.
#[derive(Debug, PartialEq, Eq)]
pub enum SnapshotError<E> {
    /// The adapter could not prove current authorization.
    Store(E),
    /// Two snapshots were invalidated; callers must not return stale data.
    Changed,
}

/// Assemble one snapshot, then check fresh authorization outside its transaction.
/// The read adapter must finish/drop its snapshot transaction before returning;
/// the check adapter must freshly authenticate token/client and return the current
/// revision. A concurrent change permits exactly one rebuild, then fails closed.
pub async fn consistent<T, E, Read, ReadFuture, Check, CheckFuture>(
    mut read: Read,
    mut check: Check,
) -> Result<T, SnapshotError<E>>
where
    Read: FnMut() -> ReadFuture,
    ReadFuture: Future<Output = Result<Snapshot<T>, E>>,
    Check: FnMut() -> CheckFuture,
    CheckFuture: Future<Output = Result<Revision, E>>,
{
    for _ in 0..2 {
        let snapshot = read().await.map_err(SnapshotError::Store)?;
        let revision = check().await.map_err(SnapshotError::Store)?;
        if revision == snapshot.revision {
            return Ok(snapshot.value);
        }
    }
    Err(SnapshotError::Changed)
}
