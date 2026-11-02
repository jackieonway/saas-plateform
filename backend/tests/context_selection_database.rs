//! Explicit, task-owned PostgreSQL test. Never uses an existing application DB
//! and never starts containers or services indirectly.
use anyhow::{anyhow, ensure, Context, Result};
use chrono::{DateTime, Duration, Utc};
use saas_platform_backend::{
    context_selection::{
        Binding, Credential, Id, Intent, Member, MemberType, Registration, Selection,
        SelectionError, Source, Version,
    },
    context_selection_store::{self as store, Consumer, ContextResult, StoreError},
    database,
};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use std::{str::FromStr, time::Duration as Timeout};
use uuid::Uuid;

const ISSUER: &str = "https://auth.saas.test";

fn id() -> Id {
    Id::new(Uuid::now_v7()).unwrap()
}
fn version(n: i64) -> Version {
    Version::new(n).unwrap()
}
fn credential() -> Credential {
    use rand::RngCore;
    let mut bytes = [0; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    Credential::parse(hex::encode(bytes)).unwrap()
}

async fn confirmed(pool: &PgPool) -> Result<(Selection, Consumer)> {
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await?;
    let user = id();
    let client = id();
    let binding = Binding {
        transaction_id: id(),
        source: Source {
            issuer: ISSUER.to_owned(),
            user_id: user,
            account_version: version(0),
            global_session_id: id(),
            interaction_id: id(),
            auth_time: now - Duration::seconds(30),
        },
        registration: Registration {
            client_id: client,
            client_policy_version: version(1),
            member_type: MemberType::Employee,
            redirect_uri: "https://platform.saas.test/auth/callback".to_owned(),
            application: None,
        },
        intent: Intent::Switch {
            session_id: id(),
            expected_version: version(1),
        },
    };
    let member = Member {
        user_id: user,
        tenant_id: id(),
        organization_id: id(),
        membership_id: id(),
        member_type: MemberType::Employee,
        identity_version: version(0),
    };
    Ok((
        Selection::confirm(
            binding,
            member,
            ISSUER,
            now,
            now + Duration::minutes(5),
            now + Duration::minutes(5),
        )?,
        Consumer {
            client_id: client,
            service_id: Some(id()),
        },
    ))
}

async fn cases(pool: PgPool) -> Result<()> {
    sqlx::query("CREATE TABLE selection_commit_markers(context_id uuid PRIMARY KEY)")
        .execute(&pool)
        .await?;
    let (selected, consumer) = confirmed(&pool).await?;
    let credential = credential();
    let credential_ciphertext = "v1.test-selection-credential-ciphertext";
    let mut tx = pool.begin().await?;
    store::issue(
        &mut tx,
        &selected,
        &credential,
        credential_ciphertext,
        consumer,
        32,
    )
    .await?;
    tx.commit().await?;
    let persisted: (String, String) =
        sqlx::query_as("SELECT credential_hash,credential_ciphertext FROM context_selections")
            .fetch_one(&pool)
            .await?;
    ensure!(
        persisted.0 == credential.digest()
            && persisted.1 == credential_ciphertext
            && !persisted.1.contains(credential.as_secret()),
        "only the digest and an encrypted credential envelope are persisted"
    );

    let mut tx = pool.begin().await?;
    let rewritten = sqlx::query("UPDATE context_selections SET expires_at=expires_at+interval '1 second' WHERE credential_hash=$1")
        .bind(credential.digest()).execute(&mut *tx).await;
    ensure!(
        rewritten.is_err(),
        "persisted source deadlines cannot be extended"
    );
    tx.rollback().await?;

    let mut tx = pool.begin().await?;
    let replaced = sqlx::query(
        "UPDATE context_selections SET credential_ciphertext='v1.forged.value' \
         WHERE credential_hash=$1",
    )
    .bind(credential.digest())
    .execute(&mut *tx)
    .await;
    ensure!(
        replaced.is_err(),
        "recoverable ciphertext cannot be replaced before consumption"
    );
    tx.rollback().await?;

    let mut tx = pool.begin().await?;
    let recovered = store::issue(
        &mut tx,
        &selected,
        &credential,
        credential_ciphertext,
        consumer,
        32,
    )
    .await?;
    ensure!(
        recovered == selected,
        "an uncertain confirmation reuses the original selection and deadline"
    );
    tx.commit().await?;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM context_selections")
        .fetch_one(&pool)
        .await?;
    ensure!(
        rows == 1,
        "recovering a confirmation inserted a second credential"
    );

    let mut tx = pool.begin().await?;
    let duplicate = store::issue(
        &mut tx,
        &selected,
        &self::credential(),
        "v1.different-selection-credential",
        consumer,
        32,
    )
    .await;
    ensure!(
        matches!(
            duplicate,
            Err(StoreError::Selection(SelectionError::Changed))
        ),
        "an interaction cannot replace its original credential with another"
    );
    tx.rollback().await?;

    let result = ContextResult::Employee {
        context_id: id(),
        version: version(2),
    };
    let mut tx = pool.begin().await?;
    let wrong = Consumer {
        client_id: consumer.client_id,
        service_id: Some(id()),
    };
    ensure!(
        matches!(
            store::consume(
                &mut tx,
                &credential,
                wrong,
                &selected.binding,
                &selected.member,
                &result,
                None
            )
            .await,
            Err(StoreError::NotFound)
        ),
        "another service cannot consume the credential"
    );
    tx.rollback().await?;

    let mut tx = pool.begin().await?;
    let customer_result = ContextResult::Customer {
        context_id: id(),
        version: version(2),
    };
    ensure!(
        matches!(
            store::consume(
                &mut tx,
                &credential,
                consumer,
                &selected.binding,
                &selected.member,
                &customer_result,
                None
            )
            .await,
            Err(StoreError::Selection(SelectionError::Changed))
        ),
        "employee selection cannot establish a customer context"
    );
    tx.rollback().await?;

    let mut tx = pool.begin().await?;
    store::consume(
        &mut tx,
        &credential,
        consumer,
        &selected.binding,
        &selected.member,
        &result,
        None,
    )
    .await?;
    tx.rollback().await?;
    let consumed: bool =
        sqlx::query_scalar("SELECT consumed_at IS NOT NULL FROM context_selections")
            .fetch_one(&pool)
            .await?;
    ensure!(
        !consumed,
        "caller rollback must restore the unused credential"
    );

    // Both callers attempt a context write. Only the winning transaction may
    // commit its context marker; no retry may manufacture a second result.
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let task_pool = pool.clone();
        let selected = selected.clone();
        let wire = serde_json::to_string(&credential)?;
        tasks.push(tokio::spawn(async move {
            let credential: Credential = serde_json::from_str(&wire)?;
            let context_id = id();
            let result = ContextResult::Employee {
                context_id,
                version: version(2),
            };
            let mut tx = task_pool.begin().await?;
            sqlx::query("INSERT INTO selection_commit_markers VALUES($1)")
                .bind(context_id.uuid())
                .execute(&mut *tx)
                .await?;
            match store::consume(
                &mut tx,
                &credential,
                consumer,
                &selected.binding,
                &selected.member,
                &result,
                None,
            )
            .await
            {
                Ok(()) => {
                    tx.commit().await?;
                    Ok::<_, anyhow::Error>(Some(result))
                }
                Err(StoreError::Selection(SelectionError::Consumed)) => {
                    tx.rollback().await?;
                    Ok(None)
                }
                Err(error) => {
                    tx.rollback().await?;
                    Err(error.into())
                }
            }
        }));
    }
    let first = tasks.remove(0).await??;
    let second = tasks.remove(0).await??;
    ensure!(
        first.is_some() != second.is_some(),
        "exactly one concurrent consume commits"
    );
    let winner = first.or(second).context("winning result missing")?;
    let credential_ciphertext: Option<String> = sqlx::query_scalar(
        "SELECT credential_ciphertext FROM context_selections WHERE credential_hash=$1",
    )
    .bind(credential.digest())
    .fetch_one(&pool)
    .await?;
    ensure!(
        credential_ciphertext.is_none(),
        "consumption clears the recoverable credential ciphertext"
    );
    let mut tx = pool.begin().await?;
    let overwritten = sqlx::query("UPDATE context_selections SET result=jsonb_set(result,'{version}','\"99\"') WHERE credential_hash=$1")
        .bind(credential.digest()).execute(&mut *tx).await;
    ensure!(
        overwritten.is_err(),
        "a committed result cannot be overwritten"
    );
    tx.rollback().await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM selection_commit_markers")
        .fetch_one(&pool)
        .await?;
    ensure!(count == 1, "losing context must roll back");

    let mut tx = pool.begin().await?;
    ensure!(
        store::recover(
            &mut tx,
            &credential,
            consumer,
            &selected.binding,
            &selected.member
        )
        .await?
            == winner,
        "recovery returns only the committed reference"
    );
    tx.commit().await?;
    let mut tx = pool.begin().await?;
    let mut changed = selected.binding.clone();
    changed.intent = Intent::Switch {
        session_id: id(),
        expected_version: version(1),
    };
    ensure!(
        matches!(
            store::recover(&mut tx, &credential, consumer, &changed, &selected.member).await,
            Err(StoreError::Selection(SelectionError::Changed))
        ),
        "another session cannot recover the result"
    );
    tx.rollback().await?;

    let (other, other_consumer) = confirmed(&pool).await?;
    let other_credential = self::credential();
    let mut tx = pool.begin().await?;
    ensure!(
        matches!(
            store::issue(
                &mut tx,
                &other,
                &other_credential,
                "v1.other-selection-credential",
                other_consumer,
                1,
            )
            .await,
            Err(StoreError::Capacity)
        ),
        "live consumed result must retain its recovery slot"
    );
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit CONTEXT_SELECTION_TEST_ADMIN_URL selecting postgres; creates and always drops its own database"]
async fn real_postgres_selection_is_single_use_and_transactional() -> Result<()> {
    let url = std::env::var("CONTEXT_SELECTION_TEST_ADMIN_URL")
        .context("explicit test administration URL required")?;
    let options =
        PgConnectOptions::from_str(&url).map_err(|_| anyhow!("invalid test administration URL"))?;
    ensure!(
        options.get_database() == Some("postgres"),
        "test administrator must select postgres"
    );
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Timeout::from_secs(5))
        .connect_with(options.clone())
        .await
        .map_err(|_| anyhow!("test administration connection failed"))?;
    let name = format!("context_selection_{}", Uuid::now_v7().simple());
    sqlx::query(&format!("CREATE DATABASE \"{name}\""))
        .execute(&admin)
        .await?;
    let task_name = name.clone();
    // Capturing panics through JoinHandle keeps cleanup outside all assertions
    // and case failures. No existing database is ever migrated or deleted.
    let result = tokio::spawn(async move {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Timeout::from_secs(5))
            .connect_with(options.database(&task_name))
            .await?;
        let result = async {
            database::run_migrations(&pool).await?;
            cases(pool.clone()).await
        }
        .await;
        pool.close().await;
        result
    });
    let result = result.await;
    // Name is generated locally, with only a fixed prefix and UUID hex digits.
    sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
        .execute(&admin)
        .await?;
    admin.close().await;
    result??;
    Ok(())
}
