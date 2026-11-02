//! Reproduce the identity service's account fence on an independent connection.
//! An in-memory SourcePort alone cannot detect a platform transaction waiting
//! for its own account lock through a remote identity call.
use super::*;

struct AccountShareSource {
    inner: Arc<Sources>,
    pool: PgPool,
    user: Uuid,
    checks: AtomicUsize,
}

impl SourcePort for AccountShareSource {
    fn verify<'a>(
        &'a self,
        transaction: &'a str,
        client: Id,
        access: Option<&'a str>,
    ) -> SourceFuture<'a> {
        Box::pin(async move {
            let proof = self.inner.verify(transaction, client, access).await?;
            // This transaction has a different connection from the consumer.
            // The identity authority reads this fence before returning a proof.
            let mut tx = self.pool.begin().await.map_err(|_| AppError::Unavailable)?;
            sqlx::query("SET LOCAL lock_timeout='300ms'")
                .execute(&mut *tx)
                .await
                .map_err(|_| AppError::Unavailable)?;
            let exists: Option<Uuid> =
                sqlx::query_scalar("SELECT id FROM users WHERE id=$1 FOR SHARE")
                    .bind(self.user)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|_| AppError::Unavailable)?;
            if exists != Some(self.user) {
                return Err(AppError::Unavailable);
            }
            tx.commit().await.map_err(|_| AppError::Unavailable)?;
            self.checks.fetch_add(1, Ordering::SeqCst);
            Ok(proof)
        })
    }
}

#[tokio::test]
#[ignore = "requires explicit task-owned IDENTITY_PHASE11_TEST_DATABASE_URL"]
async fn customer_consume_and_recovery_allow_independent_identity_account_share_fence() -> Result<()>
{
    let db = Database::create().await?;
    let result = async {
        let mut f = Fixture::create(&db.pool).await?;
        let service = Uuid::now_v7();
        sqlx::query("INSERT INTO context_selection_services(id,secret_hash,purpose,allowed_auth_clients,status) VALUES($1,$2,'CONTEXT_SELECTION_CONSUMER',ARRAY[$3]::uuid[],'ENABLED')")
            .bind(service)
            .bind(hash::sha256_hex(&"c".repeat(64)))
            .bind(f.rp)
            .execute(&db.pool)
            .await?;
        sqlx::query("INSERT INTO context_selection_registrations(client_id,client_policy_version,member_type,application_id,application_policy_version,consumer_service_id) VALUES($1,1,'CUSTOMER',$2,1,$3)")
            .bind(f.rp)
            .bind(f.app)
            .bind(service)
            .execute(&db.pool)
            .await?;
        let mut config = (*f.state.config).clone();
        config.bff_token_keys = Some(Arc::new(crate::bff::CipherKeys::fixture()));
        f.state.config = Arc::new(config);
        let sources = Arc::new(Sources(Mutex::new(HashMap::new())));
        let authority = Arc::new(AccountShareSource {
            inner: sources.clone(),
            pool: db.pool.clone(),
            user: f.user,
            checks: AtomicUsize::new(0),
        });
        f.state.context_selection_authority = authority.clone();
        let app = crate::context_selection_http::routes().with_state(f.state.clone());
        let auth = format!(
            "Basic {}",
            STANDARD.encode(format!("{service}:{}", "c".repeat(64)))
        );
        let transaction = Uuid::now_v7();
        let body = issue(
            &f,
            &sources,
            service,
            transaction,
            (&f.access, f.grant, f.sid),
            Intent::Login {},
        )
        .await?;
        let issued = call(&app, "consume", &auth, body.clone()).await?;
        expect(
            &issued,
            StatusCode::OK,
            "customer consume with an independent identity account fence",
        )?;
        ensure!(
            authority.checks.load(Ordering::SeqCst) == 2,
            "consume omitted the final independent identity source fence"
        );
        let recovered = call(&app, "result", &auth, body).await?;
        expect(
            &recovered,
            StatusCode::OK,
            "original customer recovery with an independent identity account fence",
        )?;
        ensure!(
            authority.checks.load(Ordering::SeqCst) == 4,
            "recovery omitted the final independent identity source fence"
        );
        ensure!(
            recovered.body == issued.body,
            "recovery changed the committed customer capability"
        );
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM customer_portal_contexts), \
             (SELECT count(*) FROM context_selections WHERE transaction_id=$1 AND consumed_at IS NOT NULL)",
        )
        .bind(transaction)
        .fetch_one(&db.pool)
        .await?;
        ensure!(counts == (1, 1), "recovery duplicated the original operation");
        Ok(())
    }
    .await;
    db.finish(result).await
}
