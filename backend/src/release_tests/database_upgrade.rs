//! Fresh initialization and UUIDv7 contracts, only in
//! disposable databases. No shared installation or migration record is repaired.
use super::*;
use sqlx::migrate::Migrate;
use std::time::Duration;

async fn history(pool: &PgPool) -> Result<Vec<Value>> {
    Ok(
        sqlx::query_scalar("SELECT to_jsonb(m) FROM _sqlx_migrations m ORDER BY version")
            .fetch_all(pool)
            .await?,
    )
}

async fn assert_lock_available(pool: &PgPool) -> Result<()> {
    let mut connection = pool.acquire().await?.detach();
    tokio::time::timeout(Duration::from_secs(2), connection.lock()).await??;
    connection.unlock().await?;
    sqlx::Connection::close(connection).await?;
    Ok(())
}

/// Conservative proof of a mandatory UUID predicate in a validated SQL CHECK.
/// Unrecognized syntax is not counted as a guard. In particular, a call inside
/// an OR alternative, negation or comparison never proves the enclosing CHECK.
fn check_enforces_uuid_v7(expression: &str, column: &str) -> bool {
    if expression.contains(['\'', '"', '$'])
        || !column
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return false;
    }
    fn ungroup(mut expression: &str) -> Option<&str> {
        loop {
            expression = expression.trim();
            if !expression.starts_with('(') {
                return Some(expression);
            }
            let mut depth = 0_i32;
            let mut closes_at_end = false;
            for (index, byte) in expression.bytes().enumerate() {
                match byte {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth < 0 {
                            return None;
                        }
                        if depth == 0 {
                            closes_at_end = index + 1 == expression.len();
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if !closes_at_end {
                return Some(expression);
            }
            expression = &expression[1..expression.len() - 1];
        }
    }
    fn split<'a>(expression: &'a str, operator: &str) -> Option<Vec<&'a str>> {
        let bytes = expression.as_bytes();
        let mut depth = 0_i32;
        let mut start = 0;
        let mut parts = Vec::new();
        let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        for index in 0..bytes.len() {
            match bytes[index] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth < 0 {
                        return None;
                    }
                }
                _ => {}
            }
            if depth == 0
                && bytes[index..].starts_with(operator.as_bytes())
                && (index == 0 || !word(bytes[index - 1]))
                && (index + operator.len() == bytes.len() || !word(bytes[index + operator.len()]))
            {
                parts.push(&expression[start..index]);
                start = index + operator.len();
            }
        }
        if depth != 0 {
            return None;
        }
        parts.push(&expression[start..]);
        Some(parts)
    }
    let Some(expression) = ungroup(expression) else {
        return false;
    };
    let Some(alternatives) = split(expression, "OR") else {
        return false;
    };
    if alternatives.len() > 1 {
        return alternatives
            .iter()
            .all(|part| check_enforces_uuid_v7(part, column));
    }
    let Some(conjuncts) = split(expression, "AND") else {
        return false;
    };
    if conjuncts.len() > 1 {
        return conjuncts
            .iter()
            .any(|part| check_enforces_uuid_v7(part, column));
    }
    let compact: String = expression
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    compact == format!("is_uuid_v7({column})")
        || compact == format!("public.is_uuid_v7({column})")
        // For a non-null invalid UUID this branch is false, so it is a safe
        // nullable alternative when combined with the real UUID predicate.
        || compact == format!("{column}ISNULL")
}

#[test]
fn uuid_guard_proof_rejects_optional_or_compared_function_calls() {
    for guarded in [
        "(is_uuid_v7(id))",
        "(public.is_uuid_v7(id) AND (id <> requested_by))",
        "((id IS NULL) OR public.is_uuid_v7(id))",
        "(enabled OR archived) AND is_uuid_v7(id)",
    ] {
        assert!(
            check_enforces_uuid_v7(guarded, "id"),
            "mandatory UUID guard was not recognized"
        );
    }
    for bypass in [
        "is_uuid_v7(id) OR enabled",
        "NOT is_uuid_v7(id)",
        "is_uuid_v7(id) = false",
        "enabled = (other AND is_uuid_v7(id))",
        "is_uuid_v7(other_id)",
        "'is_uuid_v7(id)' <> note",
        "(id IS NULL OR enabled) AND (is_uuid_v7(id) OR enabled)",
    ] {
        assert!(
            !check_enforces_uuid_v7(bypass, "id"),
            "optional UUID predicate counted as a guard"
        );
    }
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn canonical_initialization_has_no_bundled_microapps_and_restarts_cleanly() -> Result<()> {
    let database = TestDatabase::create_through(1).await?;
    let result = async {
        crate::database::run_migrations(&database.pool).await?;
        let before = history(&database.pool).await?;
        crate::database::run_migrations(&database.pool).await?;
        ensure!(before == history(&database.pool).await?, "idempotent startup changed migration records");
        let examples: i64 = sqlx::query_scalar("SELECT count(*) FROM applications WHERE code IN ('vue-workshop','legacy-jquery','report-center')")
            .fetch_one(&database.pool).await?;
        let menus: i64 = sqlx::query_scalar("SELECT count(*) FROM menus WHERE path LIKE '/micro/%'")
            .fetch_one(&database.pool).await?;
        ensure!(examples == 0 && menus == 0, "fresh initialization retained legacy demo records");
        let defaults: (String, String) = sqlx::query_as("INSERT INTO applications(code,name,id) VALUES('fresh-default','Fresh default',$1) RETURNING entry_type,runtime_mode")
            .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        ensure!(defaults == ("HTML".into(), "TRUSTED".into()), "fresh defaults are not qiankun");
        assert_lock_available(&database.pool).await?;
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn uuid_v7_target_enforces_versions_variants_and_application_generated_ids() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result = async {
        let uuid_columns: Vec<(String, String)> = sqlx::query_as(
            "SELECT t.relname::text,a.attname::text FROM pg_attribute a \
             JOIN pg_class t ON t.oid=a.attrelid JOIN pg_namespace n ON n.oid=t.relnamespace \
             WHERE n.nspname='public' AND t.relkind IN ('r','p') AND a.attnum>0 \
             AND a.atttypid='uuid'::regtype AND NOT a.attisdropped ORDER BY t.relname,a.attname",
        ).fetch_all(&database.pool).await?;
        // The canonical baseline is one evolving schema, not a replay of the
        // retired incremental migrations. Compare every persisted UUID column
        // with the actual validated guards below and require the core domains.
        for (table, column) in [
            ("users", "id"),
            ("memberships", "id"),
            ("auth_clients", "id"),
            ("auth_grants", "id"),
            ("platform_app_capabilities", "id"),
        ] {
            ensure!(uuid_columns.iter().any(|(actual_table, actual_column)| actual_table == table && actual_column == column),
                "canonical UUID domain is missing its required {table}.{column}");
        }
        // A referenced UUID is also guarded by a validated FK to a guarded
        // UUID. MATCH SIMPLE only proves a component when every other local
        // component is NOT NULL; otherwise a nullable companion could bypass
        // the FK. Cycles without a direct seed never enter this closure.
        let check_candidates: Vec<(i64, i16, String, String)> = sqlx::query_as(
            "SELECT c.conrelid::bigint,a.attnum,a.attname::text,pg_get_expr(c.conbin,c.conrelid) \
             FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid \
             JOIN pg_namespace n ON n.oid=t.relnamespace \
             JOIN pg_attribute a ON a.attrelid=t.oid AND a.attnum=ANY(c.conkey) \
             WHERE n.nspname='public' AND t.relkind IN ('r','p') AND c.contype='c' \
             AND c.convalidated AND a.atttypid='uuid'::regtype AND NOT a.attisdropped",
        ).fetch_all(&database.pool).await?;
        let mut direct: Vec<(i64,i16)> = check_candidates.into_iter()
            .filter(|(_,_,column,predicate)| check_enforces_uuid_v7(predicate,column))
            .map(|(relation,attribute,_,_)| (relation,attribute)).collect();
        direct.sort_unstable();
        direct.dedup();
        let (guarded_relations, guarded_attributes): (Vec<i64>,Vec<i16>) = direct.into_iter().unzip();
        let checked_uuid_columns: Vec<(String, String)> = sqlx::query_as(
            r#"WITH RECURSIVE guarded(relid, attnum) AS (
                SELECT source.relid,source.attnum
                FROM unnest($1::bigint[],$2::smallint[]) AS source(relid,attnum)
                UNION
                SELECT fk.conrelid::bigint,mapping.source_attnum
                FROM guarded target
                JOIN pg_constraint fk ON fk.confrelid::bigint=target.relid
                  AND fk.contype='f' AND fk.convalidated
                CROSS JOIN LATERAL unnest(fk.conkey,fk.confkey)
                    AS mapping(source_attnum,target_attnum)
                WHERE mapping.target_attnum=target.attnum
                  AND (fk.confmatchtype='f' OR NOT EXISTS (
                    SELECT 1 FROM unnest(fk.conkey) other(source_attnum)
                    JOIN pg_attribute sibling ON sibling.attrelid=fk.conrelid
                        AND sibling.attnum=other.source_attnum
                    WHERE other.source_attnum<>mapping.source_attnum AND NOT sibling.attnotnull
                  ))
            )
            SELECT DISTINCT t.relname::text,a.attname::text
            FROM guarded g
            JOIN pg_class t ON t.oid::bigint=g.relid
            JOIN pg_namespace n ON n.oid=t.relnamespace
            JOIN pg_attribute a ON a.attrelid=t.oid AND a.attnum=g.attnum
            WHERE n.nspname='public' AND t.relkind IN ('r','p')
              AND a.atttypid='uuid'::regtype AND NOT a.attisdropped
            ORDER BY t.relname::text,a.attname::text"#,
        ).bind(&guarded_relations).bind(&guarded_attributes).fetch_all(&database.pool).await?;
        ensure!(uuid_columns == checked_uuid_columns,
            "UUID v7 checks do not cover the exact platform UUID column set: schema={uuid_columns:?} checked={checked_uuid_columns:?}");
        let defaults: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_attribute a JOIN pg_class t ON t.oid=a.attrelid \
             JOIN pg_namespace n ON n.oid=t.relnamespace JOIN pg_attrdef d ON d.adrelid=t.oid AND d.adnum=a.attnum \
             WHERE n.nspname='public' AND t.relkind='r' AND a.atttypid='uuid'::regtype AND NOT a.attisdropped",
        ).fetch_one(&database.pool).await?;
        ensure!(defaults == 0, "persistent UUIDs still have database defaults");
        expect_sqlstate(
            sqlx::query("INSERT INTO users(username,password_hash) VALUES('missing-id','unused')")
                .execute(&database.pool).await,
            "23502", "an omitted application UUID",
        )?;
        // Intentional negative fixtures: legacy v4, nil, and both non-RFC variants.
        for invalid in [
            Uuid::new_v4(),
            Uuid::nil(),
            Uuid::parse_str("019a1163-0000-7000-0000-000000000001")?,
            Uuid::parse_str("019a1163-0000-7000-c000-000000000001")?,
        ] {
            expect_sqlstate(
                sqlx::query("INSERT INTO users(id,username,password_hash) VALUES($1,'invalid-id','unused')")
                    .bind(invalid).execute(&database.pool).await,
                "23514", "an invalid UUID version or variant",
            )?;
        }
        for invalid_reference in [Uuid::new_v4(), Uuid::nil()] {
            expect_sqlstate(
                sqlx::query("INSERT INTO auth_scopes(id,resource_id,scope_name,status) VALUES($1,$2,'owned-invalid-reference','ACTIVE')")
                    .bind(crate::util::uuid7::new_uuid()).bind(invalid_reference)
                    .execute(&database.pool).await,
                "23503", "an invalid UUID reference to a guarded canonical resource",
            )?;
        }
        let id = crate::util::uuid7::new_uuid();
        let inserted: Uuid = sqlx::query_scalar(
            "INSERT INTO users(id,username,password_hash) VALUES($1,'explicit-v7','unused') RETURNING id",
        ).bind(id).fetch_one(&database.pool).await?;
        ensure!(inserted == id, "database replaced the application UUID");
        // Nullable UUID references remain nullable after adding STRICT checks.
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(id,name,code) VALUES($1,'Nullable UUID fixture','nullable-uuid') RETURNING id",
        ).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let optional: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Nullable parent') RETURNING parent_id",
        ).bind(crate::util::uuid7::new_uuid()).bind(tenant).fetch_one(&database.pool).await?;
        ensure!(optional.is_none(), "nullable UUID result changed");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn changed_or_historical_migration_history_is_rejected_without_repair() -> Result<()> {
    let db = TestDatabase::create().await?;
    let result = async {
        let before = history(&db.pool).await?;
        ensure!(
            before.len() == 1,
            "fresh baseline must have exactly one migration"
        );
        sqlx::query(
            "UPDATE _sqlx_migrations SET checksum=decode(repeat('00',48),'hex') WHERE version=1",
        )
        .execute(&db.pool)
        .await?;
        let corrupted = history(&db.pool).await?;
        ensure!(
            crate::database::run_migrations(&db.pool).await.is_err(),
            "changed history was adopted"
        );
        ensure!(
            corrupted == history(&db.pool).await?,
            "migration history was repaired"
        );
        assert_lock_available(&db.pool).await?;
        Ok(())
    }
    .await;
    db.finish(result).await
}
