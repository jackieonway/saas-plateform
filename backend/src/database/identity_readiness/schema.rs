use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use super::{caused, fail, Failure, MembershipReadinessError};

pub(super) struct Shape {
    pub database: String,
    pub database_oid: i64,
    pub cluster_system_identifier: String,
    pub binary_key: bool,
    pub migration_sha256: String,
}

pub(super) async fn history(
    connection: &mut PgConnection,
) -> Result<String, MembershipReadinessError> {
    let rows: Vec<(i64, bool, Vec<u8>)> = sqlx::query_as(
        "SELECT version,success,checksum FROM public._sqlx_migrations ORDER BY version",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| caused(Failure::History, source))?;
    let applied: Vec<_> = rows
        .iter()
        .map(|(version, _, checksum)| sqlx::migrate::AppliedMigration {
            version: *version,
            checksum: std::borrow::Cow::Owned(checksum.clone()),
        })
        .collect();
    let migrator = super::super::compatible_migrator(&applied);
    let expected: Vec<_> = migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .collect();
    if rows.len() != expected.len()
        || rows
            .iter()
            .zip(expected)
            .any(|((version, success, checksum), migration)| {
                *version != migration.version
                    || !success
                    || checksum.as_slice() != migration.checksum.as_ref()
            })
    {
        return Err(fail(Failure::History));
    }
    let material = serde_json::to_vec(&rows).map_err(|source| caused(Failure::History, source))?;
    Ok(hex::encode(Sha256::digest(material)))
}

pub(super) async fn digest(
    connection: &mut PgConnection,
) -> Result<String, MembershipReadinessError> {
    let material: serde_json::Value = sqlx::query_scalar(include_str!("../schema_material.sql"))
        .fetch_one(&mut *connection)
        .await
        .map_err(|source| caused(Failure::Schema, source))?;
    if !material.is_array() {
        return Err(fail(Failure::Schema));
    }
    let bytes = serde_json::to_vec(&material).map_err(|source| caused(Failure::Schema, source))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn normalized_check(source: &str) -> String {
    source
        .replace("::character varying[]", "")
        .replace("::character varying", "")
        .replace("::text[]", "")
        .replace("::text", "")
        // PostgreSQL quotes this keyword-shaped column when deparsing the
        // canonical customer CHECK. Normalize only this known identifier.
        .replace("\"position\"", "position")
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '(' && *c != ')')
        .collect()
}

pub(super) async fn inspect(
    connection: &mut PgConnection,
) -> Result<Shape, MembershipReadinessError> {
    let (database, database_oid): (String, i64) = sqlx::query_as(
        "SELECT datname::text,oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database()",
    ).fetch_one(&mut *connection).await.map_err(|source| caused(Failure::Schema, source))?;
    let migration_sha256 = history(connection).await?;
    let cluster_system_identifier: String =
        sqlx::query_scalar("SELECT system_identifier::text FROM pg_catalog.pg_control_system()")
            .fetch_one(&mut *connection)
            .await
            .map_err(|source| caused(Failure::Target, source))?;
    let required_columns: bool = sqlx::query_scalar(r#"
        SELECT count(*)=7 FROM pg_catalog.pg_attribute
        WHERE attrelid='public.memberships'::regclass AND NOT attisdropped AND attnotnull
          AND ((attname IN ('id','user_id','organization_id') AND atttypid='uuid'::regtype)
            OR (attname IN ('member_type','status') AND atttypid='varchar'::regtype AND atttypmod=24)
            OR (attname='identity_version' AND atttypid='bigint'::regtype)
            OR (attname='updated_at' AND atttypid='timestamptz'::regtype))
    "#).fetch_one(&mut *connection).await.map_err(|source| caused(Failure::Schema, source))?;
    if !required_columns {
        return Err(fail(Failure::Schema));
    }
    let checks: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT conname::text,pg_catalog.pg_get_constraintdef(oid)
        FROM pg_catalog.pg_constraint WHERE conrelid='public.memberships'::regclass
          AND contype='c' AND convalidated AND conname IN (
            'memberships_member_type_check','memberships_status_check',
            'memberships_identity_version_check','memberships_customer_profile_check')
        ORDER BY conname
    "#,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| caused(Failure::Schema, source))?;
    let expected = [
        (
            "memberships_customer_profile_check",
            "CHECKmember_type<>'CUSTOMER'ORdepartment_idISNULLANDCOALESCEposition,''=''",
        ),
        (
            "memberships_identity_version_check",
            "CHECKidentity_version>=1",
        ),
        (
            "memberships_member_type_check",
            "CHECKmember_type=ANYARRAY['EMPLOYEE','CUSTOMER']",
        ),
        (
            "memberships_status_check",
            "CHECKstatus=ANYARRAY['ENABLED','DISABLED']",
        ),
    ];
    if checks.len() != expected.len()
        || checks
            .iter()
            .zip(expected)
            .any(|((name, def), (wanted, definition))| {
                name != wanted || normalized_check(def) != definition
            })
    {
        return Err(fail(Failure::Schema));
    }
    let triple_key: bool = sqlx::query_scalar(r#"
        SELECT count(*)=1 FROM pg_catalog.pg_constraint c
        JOIN pg_catalog.pg_index i ON i.indexrelid=c.conindid
        WHERE c.conrelid='public.memberships'::regclass AND c.contype='u'
          AND c.conname='memberships_user_organization_type_key' AND c.convalidated
          AND NOT c.condeferrable AND i.indisvalid AND i.indisready AND i.indisunique
          AND i.indpred IS NULL AND i.indexprs IS NULL AND i.indnkeyatts=3
          AND ARRAY(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY k(attnum,ord)
            JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.attnum ORDER BY ord)
            =ARRAY['user_id','organization_id','member_type']::text[]
    "#).fetch_one(&mut *connection).await.map_err(|source| caused(Failure::Schema, source))?;
    if !triple_key {
        return Err(fail(Failure::Schema));
    }
    let triggers: Vec<String> = sqlx::query_scalar(r#"
        SELECT p.prosrc FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_proc p ON p.oid=t.tgfoid
        JOIN pg_catalog.pg_language l ON l.oid=p.prolang
        WHERE t.tgrelid='public.memberships'::regclass AND t.tgname='trg_membership_identity_immutable'
          AND NOT t.tgisinternal AND t.tgenabled IN ('O','A') AND t.tgtype=19
          AND t.tgqual IS NULL AND t.tgnargs=0 AND t.tgconstraint=0
          AND p.oid='public.protect_membership_identity()'::regprocedure
          AND p.prorettype='trigger'::regtype AND p.pronargs=0 AND NOT p.prosecdef
          AND p.proconfig=ARRAY['search_path=pg_catalog, public']::text[] AND l.lanname='plpgsql'
    "#).fetch_all(&mut *connection).await.map_err(|source| caused(Failure::Schema, source))?;
    let canonical = include_str!("../../../migrations/0013_membership_identity_policy.sql")
        .split("LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$")
        .nth(1)
        .and_then(|s| s.split("$$;").next())
        .ok_or_else(|| fail(Failure::Schema))?;
    let normalize = |s: &str| s.replace("\r\n", "\n").trim().to_owned();
    if triggers.len() != 1 || normalize(&triggers[0]) != normalize(canonical) {
        return Err(fail(Failure::Schema));
    }
    let binary_indexes: Vec<(i64, Option<String>)> = sqlx::query_as(
        r#"
        SELECT i.indexrelid::bigint,c.conname::text FROM pg_catalog.pg_index i
        LEFT JOIN pg_catalog.pg_constraint c ON c.conindid=i.indexrelid AND c.contype='u'
        WHERE i.indrelid='public.memberships'::regclass AND i.indisunique AND i.indnkeyatts=2
          AND i.indexprs IS NULL
          AND ARRAY(SELECT a.attname::text FROM unnest(i.indkey) WITH ORDINALITY k(attnum,ord)
            JOIN pg_catalog.pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.attnum
            WHERE ord<=i.indnkeyatts ORDER BY a.attname)
            =ARRAY['organization_id','user_id']::text[]
    "#,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| caused(Failure::Schema, source))?;
    if binary_indexes.len() > 1
        || binary_indexes.first().is_some_and(|(_, name)| {
            name.as_deref() != Some("memberships_user_id_organization_id_key")
        })
    {
        return Err(fail(Failure::Schema));
    }
    if let Some((index, _)) = binary_indexes.first() {
        let dependent: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint WHERE contype='f' AND confrelid='public.memberships'::regclass AND conindid=$1::bigint::oid)",
        ).bind(index).fetch_one(&mut *connection).await.map_err(|source| caused(Failure::Schema, source))?;
        if dependent {
            return Err(fail(Failure::ForeignKey));
        }
    }
    Ok(Shape {
        database,
        database_oid,
        cluster_system_identifier,
        binary_key: !binary_indexes.is_empty(),
        migration_sha256,
    })
}

pub(super) async fn receipt_table(
    connection: &mut PgConnection,
) -> Result<bool, MembershipReadinessError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT to_regclass('public.identity_membership_readiness_receipts') IS NOT NULL",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|source| caused(Failure::Schema, source))?;
    if !exists {
        return Ok(false);
    }
    let valid: bool = sqlx::query_scalar(r#"
        SELECT r.relkind='r' AND NOT r.relrowsecurity AND NOT r.relforcerowsecurity
          AND r.relowner=m.relowner AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.aclexplode(COALESCE(r.relacl,pg_catalog.acldefault('r',r.relowner))) a
            WHERE a.grantee=0 AND a.privilege_type<>'SELECT')
        FROM pg_catalog.pg_class r CROSS JOIN pg_catalog.pg_class m
        WHERE r.oid='public.identity_membership_readiness_receipts'::regclass
          AND m.oid='public.memberships'::regclass
    "#).fetch_one(&mut *connection).await.map_err(|source| caused(Failure::Schema, source))?;
    let columns: Vec<(String, String, bool)> = sqlx::query_as(r#"
        SELECT attname::text,pg_catalog.format_type(atttypid,atttypmod),attnotnull
        FROM pg_catalog.pg_attribute WHERE attrelid='public.identity_membership_readiness_receipts'::regclass
          AND attnum>0 AND NOT attisdropped ORDER BY attnum
    "#).fetch_all(&mut *connection).await.map_err(|source| caused(Failure::Schema, source))?;
    let expected = [
        ("id", "uuid"),
        ("environment", "text"),
        ("scope", "text"),
        ("database_name", "text"),
        ("database_oid", "bigint"),
        ("cluster_system_identifier", "text"),
        ("platform_binary_sha256", "text"),
        ("manifest_sha256", "text"),
        ("migration_sha256", "text"),
        ("schema_sha256", "text"),
        ("applied_at", "timestamp with time zone"),
        ("inventory", "jsonb"),
        ("gates", "jsonb"),
    ];
    if !valid
        || columns.len() != expected.len()
        || columns
            .iter()
            .zip(expected)
            .any(|((name, ty, nonnull), (n, t))| name != n || ty != t || !nonnull)
    {
        return Err(fail(Failure::Schema));
    }
    let constraints: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT contype::text,pg_catalog.pg_get_constraintdef(oid)
        FROM pg_catalog.pg_constraint
        WHERE conrelid='public.identity_membership_readiness_receipts'::regclass
          AND (contype='p' OR conname='uuid_v7_id') AND convalidated AND NOT condeferrable
        ORDER BY contype
    "#,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| caused(Failure::Schema, source))?;
    if constraints.len() != 2
        || constraints[0].0 != "c"
        || constraints[1].0 != "p"
        || !matches!(
            normalized_check(&constraints[0].1).as_str(),
            "CHECKis_uuid_v7id" | "CHECKpublic.is_uuid_v7id"
        )
        || normalized_check(&constraints[1].1) != "PRIMARYKEYid"
    {
        return Err(fail(Failure::Schema));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::normalized_check;

    #[test]
    fn postgres_17_customer_check_deparse_preserves_the_exact_profile_guard() {
        let actual = r#"CHECK ((((member_type)::text <> 'CUSTOMER'::text) OR ((department_id IS NULL) AND ((COALESCE("position", ''::character varying))::text = ''::text))))"#;
        let expected = "CHECKmember_type<>'CUSTOMER'ORdepartment_idISNULLANDCOALESCEposition,''=''";
        assert_eq!(normalized_check(actual), expected);
        assert_ne!(normalized_check(&actual.replace("AND", "OR")), expected);
        assert_ne!(
            normalized_check(&actual.replace("\"position\"", "\"Position\"")),
            expected
        );
    }
}
