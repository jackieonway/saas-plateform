//! Fresh maintenance-database admission evidence, independent of source RR
//! snapshots. This proves application admission fences, not absence of WAL from
//! unrelated PostgreSQL activity or exclusion of the trusted local superuser.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    net::IpAddr,
    process::{Command, Stdio},
    time::Duration,
};

use super::{ExportConfig, Session, SOURCES};
use crate::{native::Scratch, Error, Result};

const FORMAT: &str = "saas-uuid-source-admission/1";
const LEGACY_ROLES: [&str; 2] = [
    "qiankun_preview_20261024",
    "qiankun_notification_preview_20261024",
];

// No Debug: the complete role/connectivity catalog is private batch evidence.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Proof {
    format: String,
    maintenance_database: String,
    maintenance_oid: u32,
    current_user: String,
    current_user_oid: u32,
    session_user: String,
    session_user_oid: u32,
    #[serde(deserialize_with = "nullable_text")]
    server_address: Option<String>,
    #[serde(deserialize_with = "nullable_port")]
    server_port: Option<u16>,
    port: u16,
    transaction_isolation: String,
    transaction_read_only: bool,
    default_transaction_read_only: bool,
    roles: Vec<Role>,
    databases: Vec<Database>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Role {
    name: String,
    oid: u32,
    login: bool,
    superuser: bool,
    inherit: bool,
    create_db: bool,
    create_role: bool,
    replication: bool,
    bypass_rls: bool,
    privileged_set: bool,
    can_admin_source_roles: bool,
    connection_limit: i32,
    source_connect: Vec<Connect>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Connect {
    kind: String,
    database: String,
    database_oid: u32,
    allowed: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Database {
    kind: String,
    name: String,
    oid: u32,
    owner: String,
    owner_oid: u32,
    allow_connections: bool,
    is_template: bool,
    connection_limit: i32,
    default_read_only: bool,
    backends: u64,
    prepared_transactions: u64,
}

fn nullable_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

fn nullable_port<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<u16>, D::Error> {
    Option::<u16>::deserialize(deserializer)
}

/// Validate authenticated source evidence without opening another connection.
/// Callers also require all six observations to be exactly equal.
pub(crate) fn validate_admission(proof: &Proof) -> Result<()> {
    let invalid = || Error("source admission evidence is not exclusively fenced");
    if proof.format != FORMAT
        || proof.maintenance_database != "postgres"
        || proof.maintenance_oid == 0
        || proof.current_user != "postgres"
        || proof.session_user != "postgres"
        || proof.current_user_oid == 0
        || proof.current_user_oid != proof.session_user_oid
        || proof.port == 0
        || proof.transaction_isolation != "read committed"
        || !proof.transaction_read_only
        || !proof.default_transaction_read_only
        || proof.databases.len() != SOURCES.len()
    {
        return Err(invalid());
    }
    match (&proof.server_address, proof.server_port) {
        (None, None) => {}
        (Some(address), Some(port))
            if port == proof.port && address.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) => {
        }
        _ => return Err(invalid()),
    }
    let mut role_oids = BTreeSet::new();
    let mut role_names = BTreeSet::new();
    for (index, role) in proof.roles.iter().enumerate() {
        if role.oid == 0
            || role.name.is_empty()
            || role.name.len() > 63
            || role.connection_limit < -1
            || !role_oids.insert(role.oid)
            || !role_names.insert(role.name.as_str())
            || index > 0 && proof.roles[index - 1].name >= role.name
            || role.source_connect.len() != SOURCES.len()
            || (role.superuser || role.create_role || LEGACY_ROLES.contains(&role.name.as_str()))
                && !role.privileged_set
        {
            return Err(invalid());
        }
    }
    let admin = proof
        .roles
        .iter()
        .find(|role| role.name == "postgres")
        .ok_or_else(invalid)?;
    if admin.oid != proof.current_user_oid || !admin.superuser || !admin.login {
        return Err(invalid());
    }
    for name in LEGACY_ROLES {
        let role = proof
            .roles
            .iter()
            .find(|role| role.name == name)
            .ok_or_else(invalid)?;
        if role.superuser || role.login {
            return Err(invalid());
        }
    }
    let mut database_oids = BTreeSet::new();
    for ((kind, name), database) in SOURCES.iter().zip(&proof.databases) {
        if database.kind != *kind
            || database.name != *name
            || database.oid == 0
            || database.oid == proof.maintenance_oid
            || !database_oids.insert(database.oid)
            || database.owner != LEGACY_ROLES[0]
            || !database.allow_connections
            || database.is_template
            || database.connection_limit < -1
            || !database.default_read_only
            || database.backends != 0
            || database.prepared_transactions != 0
        {
            return Err(invalid());
        }
        let owner = proof
            .roles
            .iter()
            .find(|role| role.name == database.owner)
            .ok_or_else(invalid)?;
        if owner.oid != database.owner_oid || owner.superuser || owner.login {
            return Err(invalid());
        }
    }
    for role in &proof.roles {
        for (database, connect) in proof.databases.iter().zip(&role.source_connect) {
            if connect.kind != database.kind
                || connect.database != database.name
                || connect.database_oid != database.oid
            {
                return Err(invalid());
            }
        }
        if role.name != "postgres"
            && role.login
            && (role.superuser
                || role.create_role
                || role.privileged_set
                || role.can_admin_source_roles
                || role.source_connect.iter().any(|connect| connect.allowed))
        {
            return Err(invalid());
        }
        if role.name == "postgres" && role.source_connect.iter().any(|connect| !connect.allowed) {
            return Err(invalid());
        }
    }
    Ok(())
}

fn script() -> String {
    let sources = SOURCES
        .iter()
        .map(|(kind, name)| format!("('{kind}'::text,'{name}'::text)"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY;
SET LOCAL search_path = pg_catalog;
SET LOCAL timezone = 'UTC';
WITH source_policy(kind,name) AS (VALUES {sources}),
source_databases AS (
 SELECT s.kind,s.name,d.oid,d.datdba,d.datallowconn,d.datistemplate,d.datconnlimit
 FROM source_policy s JOIN pg_catalog.pg_database d ON d.datname=s.name
)
SELECT jsonb_build_object(
 'format','{FORMAT}',
 'maintenance_database',current_database(),
 'maintenance_oid',(SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database()),
 'current_user',current_user,'current_user_oid',(SELECT oid::bigint FROM pg_catalog.pg_roles WHERE rolname=current_user),
 'session_user',session_user,'session_user_oid',(SELECT oid::bigint FROM pg_catalog.pg_roles WHERE rolname=session_user),
 'server_address',inet_server_addr()::text,'server_port',inet_server_port(),'port',current_setting('port')::integer,
 'transaction_isolation',current_setting('transaction_isolation'),
 'transaction_read_only',current_setting('transaction_read_only')::boolean,
 'default_transaction_read_only',current_setting('default_transaction_read_only')::boolean,
 'roles',COALESCE((SELECT jsonb_agg(jsonb_build_object(
   'name',r.rolname,'oid',r.oid::bigint,'login',r.rolcanlogin,'superuser',r.rolsuper,'inherit',r.rolinherit,
   'create_db',r.rolcreatedb,'create_role',r.rolcreaterole,'replication',r.rolreplication,
   'bypass_rls',r.rolbypassrls,'connection_limit',r.rolconnlimit,
   'privileged_set',EXISTS(SELECT 1 FROM pg_catalog.pg_roles target
      WHERE (target.rolsuper OR target.rolcreaterole OR target.rolname IN
        ('qiankun_preview_20261024','qiankun_notification_preview_20261024'))
      AND pg_catalog.pg_has_role(r.oid,target.oid,'SET')),
   'can_admin_source_roles',EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members grant_edge
      JOIN pg_catalog.pg_roles source_role ON source_role.oid=grant_edge.roleid
      WHERE source_role.rolname IN ('qiankun_preview_20261024','qiankun_notification_preview_20261024')
        AND grant_edge.admin_option
        AND pg_catalog.pg_has_role(r.oid,grant_edge.member,'SET')),
   'source_connect',(SELECT jsonb_agg(jsonb_build_object(
       'kind',d.kind,'database',d.name,'database_oid',d.oid::bigint,
       'allowed',pg_catalog.has_database_privilege(r.oid,d.oid,'CONNECT')) ORDER BY d.kind COLLATE "C")
       FROM source_databases d)
 ) ORDER BY r.rolname::text COLLATE "C") FROM pg_catalog.pg_roles r),'[]'::jsonb),
 'databases',COALESCE((SELECT jsonb_agg(jsonb_build_object(
   'kind',d.kind,'name',d.name,'oid',d.oid::bigint,'owner',owner.rolname,'owner_oid',owner.oid::bigint,
   'allow_connections',d.datallowconn,'is_template',d.datistemplate,'connection_limit',d.datconnlimit,
   'default_read_only',EXISTS(SELECT 1 FROM pg_catalog.pg_db_role_setting setting
      WHERE setting.setdatabase=d.oid AND setting.setrole=0
        AND setting.setconfig @> ARRAY['default_transaction_read_only=on']),
   'backends',(SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datid=d.oid),
   'prepared_transactions',(SELECT count(*) FROM pg_catalog.pg_prepared_xacts WHERE database=d.name)
 ) ORDER BY d.kind COLLATE "C") FROM source_databases d JOIN pg_catalog.pg_roles owner ON owner.oid=d.datdba),'[]'::jsonb)
);
COMMIT;
"#
    )
}

/// Observe only when this tool has no source sessions: before opening a source
/// RR transaction and after that source session has completely finished.
pub(super) fn observe(config: &ExportConfig, scratch: &Scratch) -> Result<Proof> {
    config.validate_connection()?;
    let stderr = super::new_plaintext(
        &scratch
            .0
            .join(format!("admission-{}.stderr", uuid::Uuid::now_v7())),
    )?;
    let mut command = Command::new("psql");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("PG") {
            command.env_remove(name);
        }
    }
    command
        .args([
            "-X",
            "-q",
            "-A",
            "-t",
            "--no-password",
            "-v",
            "ON_ERROR_STOP=1",
            "--host",
            &config.host,
            "--port",
            &config.port.to_string(),
            "--username",
            &config.user,
            "--dbname",
            "postgres",
            "--file=-",
        ])
        .env("PGPASSFILE", "/dev/null")
        .env("PGCONNECT_TIMEOUT", "10")
        .env(
            "PGOPTIONS",
            format!(
                "-c default_transaction_read_only=on -c statement_timeout={} -c lock_timeout=3000",
                config.timeout_seconds * 1000
            ),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr));
    let mut session = Session::start_process(command, Duration::from_secs(config.timeout_seconds))?;
    session.send(script(), true)?;
    let proof: Proof = super::strict_json(
        &session
            .line()?
            .ok_or(Error("fresh source admission evidence missing"))?,
    )?;
    session.finish()?;
    validate_admission(&proof)?;
    if proof.port != config.port
        || config.host.starts_with('/') && proof.server_address.is_some()
        || !config.host.starts_with('/')
            && proof.server_address.as_deref() != Some(config.host.as_str())
    {
        return Err(Error("fresh source admission transport mismatch"));
    }
    Ok(proof)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fixture() -> Value {
        let databases: Vec<_> = SOURCES.iter().enumerate().map(|(n,(kind,name))| json!({
            "kind":kind,"name":name,"oid":100+n,"owner":LEGACY_ROLES[0],"owner_oid":20,
            "allow_connections":true,"is_template":false,"connection_limit":-1,"default_read_only":true,
            "backends":0,"prepared_transactions":0
        })).collect();
        let connects = |allowed: bool| -> Value {
            json!(databases.iter().map(|db| json!({
            "kind":db["kind"],"database":db["name"],"database_oid":db["oid"],"allowed":allowed
        })).collect::<Vec<_>>())
        };
        let role = |name: &str, oid: u32, login: bool, superuser: bool, allowed: bool| {
            json!({
                "name":name,"oid":oid,"login":login,"superuser":superuser,"inherit":true,
            "create_db":superuser,"create_role":superuser,"replication":superuser,"bypass_rls":superuser,
            "privileged_set":true,
            "can_admin_source_roles":false,
                "connection_limit":-1,"source_connect":connects(allowed)
            })
        };
        json!({"format":FORMAT,"maintenance_database":"postgres","maintenance_oid":5,
            "current_user":"postgres","current_user_oid":10,"session_user":"postgres","session_user_oid":10,
            "server_address":null,"server_port":null,"port":5432,"transaction_isolation":"read committed",
            "transaction_read_only":true,"default_transaction_read_only":true,
            "roles":[role("postgres",10,true,true,true),role(LEGACY_ROLES[1],21,false,false,true),role(LEGACY_ROLES[0],20,false,false,true)],
            "databases":databases})
    }

    fn parse(value: &Value) -> Proof {
        super::super::strict_json(&serde_json::to_vec(value).unwrap()).unwrap()
    }
    fn accepts(value: &Value) -> bool {
        validate_admission(&parse(value)).is_ok()
    }

    #[test]
    fn valid_proof_is_strict_stable_and_binds_every_source_oid_and_role_flag() {
        let value = fixture();
        let original = parse(&value);
        validate_admission(&original).unwrap();
        let loaded: Proof =
            super::super::strict_json(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert!(loaded == original);
        for pointer in [
            "/roles/1/oid",
            "/roles/1/inherit",
            "/roles/1/create_db",
            "/databases/0/oid",
            "/databases/0/connection_limit",
        ] {
            let mut changed = value.clone();
            let current = changed.pointer_mut(pointer).unwrap();
            *current = match current {
                Value::Bool(value) => json!(!*value),
                _ => json!(900),
            };
            assert!(parse(&changed) != original);
        }
        let mut tcp = value.clone();
        tcp["server_address"] = json!("127.0.0.1");
        tcp["server_port"] = json!(5432);
        assert!(accepts(&tcp));
    }

    #[test]
    fn postgres_oid_json_strings_require_numeric_sql_projection_not_loose_dto_parsing() {
        // PostgreSQL serializes its oid type as a JSON string. SQL must cast
        // every wire OID to bigint; accepting strings here would hide that bug.
        let numeric = fixture();
        assert!(accepts(&numeric));
        for pointer in [
            "/maintenance_oid",
            "/current_user_oid",
            "/session_user_oid",
            "/roles/0/oid",
            "/roles/0/source_connect/0/database_oid",
            "/databases/0/oid",
            "/databases/0/owner_oid",
        ] {
            for invalid in [
                json!(numeric.pointer(pointer).unwrap().to_string()),
                json!(-1),
                json!(u64::from(u32::MAX) + 1),
            ] {
                let mut old_wire = numeric.clone();
                *old_wire.pointer_mut(pointer).unwrap() = invalid;
                assert!(super::super::strict_json::<Proof>(
                    &serde_json::to_vec(&old_wire).unwrap()
                )
                .is_err());
            }
        }
        let sql = script();
        for projection in [
            "'maintenance_oid',(SELECT oid::bigint",
            "'current_user_oid',(SELECT oid::bigint",
            "'session_user_oid',(SELECT oid::bigint",
            "'oid',r.oid::bigint",
            "'database_oid',d.oid::bigint",
            "'oid',d.oid::bigint",
            "'owner_oid',owner.oid::bigint",
        ] {
            assert!(sql.contains(projection));
        }
    }

    #[test]
    fn any_other_login_superuser_or_public_inherited_connect_permission_is_rejected() {
        for superuser in [false, true] {
            let mut value = fixture();
            let mut role = value["roles"][1].clone();
            role["name"] = json!("unrelated_login");
            role["oid"] = json!(30);
            role["login"] = json!(true);
            role["superuser"] = json!(superuser);
            role["privileged_set"] = json!(superuser);
            for connect in role["source_connect"].as_array_mut().unwrap() {
                connect["allowed"] = json!(false);
            }
            value["roles"].as_array_mut().unwrap().push(role);
            assert_eq!(accepts(&value), !superuser);
            let mut settable = value.clone();
            settable["roles"][3]["inherit"] = json!(false);
            settable["roles"][3]["privileged_set"] = json!(true);
            assert!(!accepts(&settable));
            let mut administrator = value.clone();
            administrator["roles"][3]["inherit"] = json!(false);
            administrator["roles"][3]["can_admin_source_roles"] = json!(true);
            assert!(!accepts(&administrator));
            let mut role_creator = value.clone();
            role_creator["roles"][3]["create_role"] = json!(true);
            assert!(!accepts(&role_creator));
            for source in 0..SOURCES.len() {
                let mut inherited = value.clone();
                inherited["roles"][3]["source_connect"][source]["allowed"] = json!(true);
                assert!(!accepts(&inherited));
            }
        }
    }

    #[test]
    fn old_roles_owner_fences_backends_prepared_and_source_coverage_cannot_drift() {
        for (pointer, bad) in [
            ("/roles/1/login", json!(true)),
            ("/roles/2/superuser", json!(true)),
            ("/databases/0/owner", json!("postgres")),
            ("/databases/0/owner_oid", json!(21)),
            ("/databases/0/default_read_only", json!(false)),
            ("/databases/1/backends", json!(1)),
            ("/databases/2/prepared_transactions", json!(1)),
            ("/databases/1/allow_connections", json!(false)),
            ("/databases/0/is_template", json!(true)),
            ("/databases/0/oid", json!(0)),
            ("/databases/1/oid", json!(100)),
            ("/databases/0/name", json!("unreviewed_private_db")),
            ("/roles/0/source_connect/0/database_oid", json!(999)),
        ] {
            let mut value = fixture();
            *value.pointer_mut(pointer).unwrap() = bad;
            assert!(!accepts(&value));
        }
        let mut value = fixture();
        value["databases"][0]["owner"] = json!(LEGACY_ROLES[1]);
        value["databases"][0]["owner_oid"] = json!(21);
        assert!(!accepts(&value));
        let mut value = fixture();
        value["roles"].as_array_mut().unwrap().remove(1);
        assert!(!accepts(&value));
        let mut value = fixture();
        value["databases"].as_array_mut().unwrap().pop();
        assert!(!accepts(&value));
        let mut value = fixture();
        value["roles"][2]["source_connect"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(!accepts(&value));
    }

    #[test]
    fn maintenance_identity_fresh_isolation_local_transport_and_unknown_fields_fail_closed() {
        for (pointer, bad) in [
            ("/maintenance_database", json!(SOURCES[0].1)),
            ("/current_user", json!(LEGACY_ROLES[0])),
            ("/session_user", json!(LEGACY_ROLES[0])),
            ("/session_user_oid", json!(99)),
            ("/transaction_isolation", json!("repeatable read")),
            ("/transaction_read_only", json!(false)),
            ("/default_transaction_read_only", json!(false)),
            ("/server_address", json!("198.51.100.1")),
            ("/server_port", json!(5432)),
            ("/port", json!(0)),
        ] {
            let mut value = fixture();
            *value.pointer_mut(pointer).unwrap() = bad;
            assert!(!accepts(&value));
        }
        for pointer in ["", "/roles/0", "/databases/0", "/roles/0/source_connect/0"] {
            let mut value = fixture();
            value
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("private_unknown".into(), json!("private-unlogged-value"));
            let error = super::super::strict_json::<Proof>(&serde_json::to_vec(&value).unwrap())
                .err()
                .unwrap();
            assert!(!error.to_string().contains("private-unlogged-value"));
        }
        for field in ["server_address", "server_port", "roles", "databases"] {
            let mut value = fixture();
            value.as_object_mut().unwrap().remove(field);
            assert!(
                super::super::strict_json::<Proof>(&serde_json::to_vec(&value).unwrap()).is_err()
            );
        }
        let duplicate = br#"{"format":"private","format":"private"}"#;
        assert!(super::super::strict_json::<Proof>(duplicate).is_err());
    }

    #[test]
    fn observer_uses_a_fresh_read_committed_read_only_maintenance_statement_and_effective_acl() {
        let sql = script();
        assert!(sql.starts_with("BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY;"));
        assert!(!sql.contains("REPEATABLE READ"));
        assert!(!sql.contains("pg_current_wal"));
        assert!(sql.contains("has_database_privilege(r.oid,d.oid,'CONNECT')"));
        assert!(sql.contains("pg_has_role(r.oid,target.oid,'SET')"));
        assert!(sql.contains("target.rolsuper OR target.rolcreaterole"));
        assert!(sql.contains("pg_catalog.pg_auth_members grant_edge"));
        assert!(sql.contains("grant_edge.admin_option"));
        assert!(sql.contains("pg_has_role(r.oid,grant_edge.member,'SET')"));
        assert!(sql.contains("pg_catalog.pg_roles"));
        assert!(sql.contains("pg_catalog.pg_db_role_setting"));
        assert!(sql.contains("pg_catalog.pg_stat_activity WHERE datid=d.oid"));
        assert!(sql.ends_with("COMMIT;\n"));
    }
}
