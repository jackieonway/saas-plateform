//! Phase 9 OIDC contracts use real HTTP handlers and owned disposable databases.
//! Only IDENTITY_PHASE09_TEST_DATABASE_URL is read; no dotenv or existing database is migrated.
use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::{middleware, Router};
use base64::Engine;
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::state::AppState;
use crate::util::{hash, password, uuid7};

const SECRET: &str = "identity-phase09-owned-fixture-signing-key";
const PASSWORD: &str = "Owned-Phase09-Fixture-Password9!";
const REDIRECT: &str = "https://console.example.invalid/auth/callback";
const CUSTOMER_REDIRECT: &str = "https://customer.example.invalid/auth/callback";
const BROWSER_COOKIE: &str = "__Host-saas_oidc_browser";
const VERIFIER: &str = "Owned-phase09-code-verifier-0123456789012345678901234567890123456789";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let raw = std::env::var("IDENTITY_PHASE09_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE09_TEST_DATABASE_URL is required"))?;
        let url = reqwest::Url::parse(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 9 database configuration"))?;
        let port = url
            .port()
            .ok_or_else(|| anyhow!("Phase 9 administration requires an explicit port"))?;
        let loopback = |host: &str| {
            matches!(
                host.trim_matches(&['[', ']'][..]),
                "127.0.0.1" | "localhost" | "::1"
            )
        };
        ensure!(matches!(url.scheme(),"postgres"|"postgresql") &&
                url.host_str().is_some_and(loopback) && port>0 && !url.username().is_empty() &&
                url.password().is_some_and(|value| !value.is_empty()) && url.fragment().is_none() &&
                url.query_pairs().all(|(key,value)| matches!(key.as_ref(),"sslmode"|"ssl-mode") &&
                    matches!(value.as_ref(),"disable"|"prefer"|"require")),
                "Phase 9 administration requires an explicit loopback host, port and account without redirection");
        let options = PgConnectOptions::from_str(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 9 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres")
                && loopback(options.get_host())
                && options.get_port() == port
                && !options.get_username().is_empty(),
            "Phase 9 administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated Phase 9 administration connection failed"))?;
        let name = format!("identity_phase09_{}", uuid7::new_uuid().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = match PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                admin.close().await;
                return Err(anyhow!("owned Phase 9 database connection failed"));
            }
        };
        let database = Self { pool, admin, name };
        if let Err(error) = sqlx::migrate!("./migrations").run(&database.pool).await {
            let _ = database.finish(Ok(())).await;
            return Err(error.into());
        }
        Ok(database)
    }

    async fn finish(self, assertions: Result<()>) -> Result<()> {
        let suffix = self.name.strip_prefix("identity_phase09_");
        ensure!(
            suffix.is_some_and(|value| value.len() == 32
                && value.bytes().all(|ch| ch.is_ascii_hexdigit())
                && Uuid::parse_str(value).is_ok_and(uuid7::is_uuid_v7)),
            "refusing cleanup outside an owned Phase 9 database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!(
            "identity_phase09_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
            self.name, assertions.is_ok(), removed.is_ok()
        );
        assertions?;
        removed?;
        ensure!(closed, "owned Phase 9 pool did not close");
        Ok(())
    }
}

fn router(state: AppState) -> Router {
    crate::routes::api_router()
        .nest("/oauth", crate::routes::oauth2::oauth_router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::observability::observe,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::normalize_client_ip,
        ))
        .with_state(state)
}

struct Reply {
    status: StatusCode,
    body: Value,
    headers: HeaderMap,
}

async fn request(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    content_type: &str,
    body: String,
) -> Result<Reply> {
    exchange(
        app,
        method,
        path,
        bearer,
        body,
        &[("content-type", content_type)],
    )
    .await
}

async fn exchange(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    body: String,
    headers: &[(&str, &str)],
) -> Result<Reply> {
    static SOURCE: AtomicU16 = AtomicU16::new(1);
    let source = SOURCE.fetch_add(1, Ordering::SeqCst);
    let peer = SocketAddr::from(([192, 0, (source >> 8) as u8, source as u8], 30038));
    exchange_from(app, method, path, bearer, body, headers, peer).await
}

async fn exchange_from(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    body: String,
    headers: &[(&str, &str)],
    peer: SocketAddr,
) -> Result<Reply> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("x-request-id", uuid7::new_uuid().to_string())
        .extension(ConnectInfo(peer));
    if let Some(token) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = tokio::time::timeout(
        Duration::from_secs(25),
        app.clone().oneshot(request.body(Body::from(body))?),
    )
    .await
    .map_err(|_| anyhow!("owned Phase 9 HTTP request timed out"))??;
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
        headers,
    })
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    body: Value,
) -> Result<Reply> {
    request(
        app,
        method,
        path,
        bearer,
        "application/json",
        body.to_string(),
    )
    .await
}

async fn form(app: &Router, path: &str, fields: &[(&str, String)]) -> Result<Reply> {
    let encoded = fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encoding::utf8_percent_encode(key, percent_encoding::NON_ALPHANUMERIC),
                percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    request(
        app,
        Method::POST,
        path,
        None,
        "application/x-www-form-urlencoded",
        encoded,
    )
    .await
}

fn expect(reply: &Reply, status: StatusCode, label: &str) -> Result<()> {
    // Never include a token-bearing response body, plaintext password or PHC hash.
    ensure!(
        reply.status == status,
        "{label}: expected {status}, received {}",
        reply.status
    );
    Ok(())
}

fn text<'a>(body: &'a Value, name: &str) -> Result<&'a str> {
    body[name]
        .as_str()
        .ok_or_else(|| anyhow!("missing public response field {name}"))
}

#[derive(Clone)]
struct Account {
    id: Uuid,
    username: String,
}

async fn account(pool: &PgPool, name: &str, administrator: bool) -> Result<Account> {
    let id = uuid7::new_uuid();
    let encoded = password::hash_password(PASSWORD)?;
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin,real_name) VALUES($1,$2,$3,$4,'Owned Phase 9 account')")
        .bind(id).bind(name).bind(encoded).bind(administrator).execute(pool).await?;
    Ok(Account {
        id,
        username: name.into(),
    })
}
const ORIGIN: &str = "https://auth.example.invalid";
const SESSION_COOKIE: &str = "__Host-saas_sso";
const PREAUTH_COOKIE: &str = "__Host-saas_sso_preauth";

struct Sso {
    cookie: String,
    csrf: String,
}

fn no_store(reply: &Reply) -> Result<()> {
    ensure!(
        reply
            .headers
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.split(',').any(|part| part.trim() == "no-store")),
        "SSO success or error response is cacheable"
    );
    Ok(())
}

fn response_cookie(reply: &Reply, name: &str) -> Result<String> {
    let prefix = format!("{name}=");
    let cookies = reply
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter(|value| value.starts_with(&prefix))
        .collect::<Vec<_>>();
    ensure!(
        cookies.len() == 1,
        "SSO response omitted or repeated its expected cookie"
    );
    let raw = cookies[0];
    let attributes = raw.split(';').map(str::trim).collect::<Vec<_>>();
    let same_site = if name == BROWSER_COOKIE {
        "SameSite=None"
    } else {
        "SameSite=Lax"
    };
    ensure!(
        attributes.contains(&"Secure")
            && attributes.contains(&"HttpOnly")
            && attributes.contains(&same_site)
            && attributes.contains(&"Path=/")
            && !attributes
                .iter()
                .any(|part| part.to_ascii_lowercase().starts_with("domain=")),
        "SSO cookie broadened its origin or weakened browser protections"
    );
    Ok(attributes[0].to_owned())
}

fn cookie_secret(cookie: &str) -> Result<&str> {
    let value = cookie
        .split_once('=')
        .map(|(_, value)| value)
        .ok_or_else(|| anyhow!("owned SSO fixture cookie has no value"))?;
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch)),
        "SSO cookie is not a canonical 256-bit opaque value"
    );
    Ok(value)
}

async fn browser(
    app: &Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    body: Value,
) -> Result<Reply> {
    let mut headers = vec![("origin", ORIGIN), ("content-type", "application/json")];
    if let Some(cookie) = cookie {
        headers.push(("cookie", cookie));
    }
    if let Some(csrf) = csrf {
        headers.push(("x-csrf-token", csrf));
    }
    let reply = exchange(app, method, path, None, body.to_string(), &headers).await?;
    no_store(&reply)?;
    Ok(reply)
}

async fn challenge(app: &Router) -> Result<Sso> {
    let reply = browser(
        app,
        Method::GET,
        "/api/sso/preauth",
        None,
        None,
        Value::Null,
    )
    .await?;
    expect(
        &reply,
        StatusCode::OK,
        "owned SSO preauthentication challenge",
    )?;
    let cookie = response_cookie(&reply, PREAUTH_COOKIE)?;
    cookie_secret(&cookie)?;
    Ok(Sso {
        cookie,
        csrf: text(&reply.body, "csrfToken")?.to_owned(),
    })
}

async fn sso_login(app: &Router, account: &Account, supplied: &str) -> Result<Sso> {
    let proof = challenge(app).await?;
    let reply = browser(
        app,
        Method::POST,
        "/api/sso/login",
        Some(&proof.cookie),
        Some(&proof.csrf),
        json!({"username":account.username,"password":supplied}),
    )
    .await?;
    expect(
        &reply,
        StatusCode::OK,
        "actual member-independent SSO password authentication",
    )?;
    ensure!(
        reply.body["contractVersion"] == "sso-session/1"
            && reply.body["account"]["id"] == json!(account.id)
            && reply.body["account"]["username"] == account.username,
        "SSO login changed the stable account contract"
    );
    let cookie = response_cookie(&reply, SESSION_COOKIE)?;
    cookie_secret(&cookie)?;
    ensure!(
        cookie_secret(&cookie)? != cookie_secret(&proof.cookie)?,
        "SSO login reused its preauthentication secret"
    );
    let cleared = response_cookie(&reply, PREAUTH_COOKIE)?;
    ensure!(
        cleared == format!("{PREAUTH_COOKIE}="),
        "successful login retained its browser challenge"
    );
    Ok(Sso {
        cookie,
        csrf: text(&reply.body, "csrfToken")?.to_owned(),
    })
}

async fn sso_session(app: &Router, sso: &Sso) -> Result<Reply> {
    browser(
        app,
        Method::GET,
        "/api/sso/session",
        Some(&sso.cookie),
        None,
        Value::Null,
    )
    .await
}

async fn session_id(pool: &PgPool, sso: &Sso) -> Result<Uuid> {
    Ok(
        sqlx::query_scalar("SELECT id FROM sso_sessions WHERE token_hash=$1")
            .bind(hash::sha256_hex(cookie_secret(&sso.cookie)?))
            .fetch_one(pool)
            .await?,
    )
}
// Generated test-only RSA fixture; never deployment key material.
const KEY_A_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC09dwNsQP7vfcH
jnpvava5qClcjTXKqyuhI6cBa8pWoXIO6nzlwICoOjaApNkNMXhblq2wD3m1gKWG
Tz25I7EYJ6F7BCLIhBgOjv3vCxXxcy+LIC3kEpAwFknr75n3GVF7aLUeb6Cnbfed
KTw5FBgpf8BfOa6pHAlZUI25yYcIWFMZeZ02GY0jLZagrKf+ozbpYmfxvN9Ov4q7
X1t2LuC9vMnzWKl2PXamHGRmsV/X/xhzOfEXXEUICs6zv9ZRNCnHjFmbhmd6InBb
lJu2OAnWJaGulnZvv95EAIqezbZzbpSwrcOLX/pSMHdujFSL/1yQuzoANE2ZgCMd
HIyPGOElAgMBAAECggEBAI+7MFuXnAhaLFnOKyVQGkQAc0nVx0IAjxjgTofsa7w/
Y27BhQ6fxhDYm+22Ache4LAukCplQFCPq+lwCyCAXutGnxYshMwrT0hQ/xTdA0LS
MJMarhiXPftT2McTeoEvlO6mjiVsmHgj+ibUV8cmVMKxe8Sm9jnP0QUl3EwS67QN
zY+KNkFoFJ1i9NCwQGL5+eAkgIM6aIij446+ABtN9gBLyCJZxOL7EQB6EMPpG9wE
MWmXnMObm2x3BhvbT5tru9QqZtEWe9CvJKXzgqkE+M3mffJwYfc6TWYRrMbdhv47
ZyTDwIIBzcmTkmQZQ9mBmCR6eB+YPCEFKmcF0rPelYkCgYEA1AH+gt8+KyixWvWE
xgoWmFkWJTyvoCpRziSCnK4G5Ym47GkXWqIzrHPRVivrI78S+OdEH2sdZK8BeD2Y
CMCFBrTCqJAabdSyh0hroUUnNBo3jtDY+b5U5FXmRe7m2b5276K39mnGf7UfexcD
QQ5iVNUUVBiEB7tZbfvlxeqKPFsCgYEA2oKa7ti9KCLLig2MnfaVY4wlMGJbQbRO
hDTjf5RcGvZQuKeWG3idCK+5xcw/nwgSuOZggfneeCA2SQBsESn7AVs1cIDx7Sif
y4q+oQzPUY3wygbGK5P1OigttbWQ2PhuHKOZiK8jHUD8RXlIxN5DmoOutHCVWnK/
B5zt5x9h0H8CgYBy0hmre4jKCyHy52Hfd/tAy47o3/MmRYVEphVvEfl+O5bBwzD4
zzw5PAi2KNcS5xs5XtdQRM0IiJe9LW633rytIx0B+fm2XinKhvhzwWY4G7LXUFkS
+Xg0qikq2vaAG12C75e+hw8Qf2HrYk1K4KV02GpIGXb4UQlgPbr1886mXQKBgBNW
i6+nrtXmfmzxVgiE0iPFO2VUzw65jllU9O1Ni/k24130Nuynd/5UzvQaZ2cDp+SF
PvVBz9aBw0wipW2/cV9lWw1h3AdJGRtCZCyMnaxyC2VVxCQHLCTBeYojT3ZkRhpy
wBvApDLqlWjwmugRY5pBHS0K6azfith5ovRZaqaxAoGAUZsYvxogPk1EkcTBih/A
1mKwSQGUwBIuIP+Ei/HM3cyyJgASldjw5qpNA90Rm0Y3vFE+/ibzZ4mfYjcHGQ42
5CpRpKojUVIgB1WVM7ZeP4FFM8CjsrjEsXOv4ttWp2OfjpxswiTb+BQTboPh2UWK
oeOPG3v0ydLSAGjg4ngsej0=
-----END PRIVATE KEY-----"#;
const KEY_A_N: &str = "tPXcDbED-733B456b2r2uagpXI01yqsroSOnAWvKVqFyDup85cCAqDo2gKTZDTF4W5atsA95tYClhk89uSOxGCehewQiyIQYDo797wsV8XMviyAt5BKQMBZJ6--Z9xlRe2i1Hm-gp233nSk8ORQYKX_AXzmuqRwJWVCNucmHCFhTGXmdNhmNIy2WoKyn_qM26WJn8bzfTr-Ku19bdi7gvbzJ81ipdj12phxkZrFf1_8YcznxF1xFCArOs7_WUTQpx4xZm4ZneiJwW5SbtjgJ1iWhrpZ2b7_eRACKns22c26UsK3Di1_6UjB3boxUi_9ckLs6ADRNmYAjHRyMjxjhJQ";

// Generated test-only RSA fixture; never deployment key material.
const KEY_B_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDSdBDrfXYib7qY
IPd+TxGlIB/JT+5DH76bnqkxI2plX44y9bHKgJG7x749ERrbdJuGLTraPxTgOCJC
OVDFR7CJjyH//lo5LJ0wuukASSbsZ9Oaqm+sBqQLc36brXuN+ffeLYHnshqktQsn
ONolUcOAsGfo0mbkzIdFZrc69eiGXu+Ox0XsSwa5nsG+9RlVutSlpb7sEWVgFqBc
WrLVxiW2K257csOyF+a4z5qO9Ku8VFHhZuoKyfa85oKgxrY+2AJq6QCjFoJudvb+
/a7xIkJjUBYjq9oTLMD1C0OKifiNo00M67UHqLNVqR+gXBH5muJb7DOo/lejIIvF
5REGcxxpAgMBAAECggEAJ8y/bmmZr8WFekOxyndVlO7QVBgDrToRqAiHYr2Z33YE
xGpmpy9NYD/UIGRduecBiox/Nsr7PbQf9no0Zjq6jik9etY1Zd6kE5Ulf2JWNBsm
j3dwCw5uc6Px66sRVLZTQ/dDO/gYxdFfPJmp16jw2iOCYyt3PgnYAy37tPDQeqid
mRYJx2fuPayFwAJ8b801E2tAwVYMoP4WZZIgbd6h46SmjLSYUYKPCxNT1pcWdqWC
7Be6NnbtLuwuaRDLz6DjmVhduRp7imb3OKy1DNUSAvtuGZPeP8lZwMXjVq0Xlblp
ppjxT8YvFzXDf+TNi+X23QFUxMZdyqxmtyIafK9tyQKBgQD3UemQtL15WmpLQW7l
fjo8IH7RNLFO2CssBKC7bnSjtIr0dlawyyuTwrzlqe7ln+DfSbDuzPHZpfoQSNr+
LYgECc4X3zXfYSwa7x59I8qmnvfR1Tvhztu4Qxfq8lQLWlV4tuiKzEBO5yTpmuD0
24skk6jXwToR7ZNznf277XN53wKBgQDZ1utyQ7REbRHGKrvF05XPqA1vv76l5cMQ
yrk5C7YlDW4ysLr5NHHJSBD+eFDlQ7zS2pQu6rROC54i1i51V7tqm4TD9/ct1hdl
De6GvuGRRefMPOGTpF8MNDWbAojUulOqxxEBfDvsr4VviE3ui9VFgggVWFrrV5Dy
51GXIvTCtwKBgQCgdLVbG9f93ootzwL6w31WuyJr6orKg3bm9Q2u7Meh2iV42zn9
M6NPFz3kpu6GbcbANxgipjaWpZdqb0odND8Vl6LtW/Sgu3OY+eBHAiydE2/m5DgY
lVN55ZWj6ZpbTz06tziBrGhgiJgrMz+VvMY2cdJOQF/Dd+Z9UVhoT3Lm8wKBgDi1
l4Oeln49uqPRGiLs6xItGJpf21zKzJZBgCiWoOtSbL6rfXFEst6IcUwdvEmC8dQe
bPctyJSMt7Uv9tY7aaHeLJjro6B+HIcFMnRHmuc7wJVNwBe0cb32lYHuDfr7Kq7J
teaiFeOO6fjYDhmss2+t4zbTUo/zsttZVsc2pWtPAoGBANK7fR6e7xzJO9HUcps6
BxJYVG6QPjiOByleChe6aV/qC2cbpKcLuX2+KgJ2QvEFpNGaKyWOsrdw8AUdrb3n
NThAK0Aigvp0YHt9Xz9aym0ZQZQBJFn/ekZfkoM3ypBypHdcOoCBTnz24GRi0Jzi
l3u7+IIb9K0sy+4phDmSFSRe
-----END PRIVATE KEY-----"#;
const KEY_B_N: &str = "0nQQ6312Im-6mCD3fk8RpSAfyU_uQx--m56pMSNqZV-OMvWxyoCRu8e-PREa23Sbhi062j8U4DgiQjlQxUewiY8h__5aOSydMLrpAEkm7GfTmqpvrAakC3N-m617jfn33i2B57IapLULJzjaJVHDgLBn6NJm5MyHRWa3OvXohl7vjsdF7EsGuZ7BvvUZVbrUpaW-7BFlYBagXFqy1cYltitue3LDshfmuM-ajvSrvFRR4WbqCsn2vOaCoMa2PtgCaukAoxaCbnb2_v2u8SJCY1AWI6vaEyzA9QtDion4jaNNDOu1B6izVakfoFwR-ZriW-wzqP5XoyCLxeURBnMcaQ";

struct KeyFile {
    path: PathBuf,
}

impl KeyFile {
    fn new() -> Result<Self> {
        let artifact = Self {
            path: std::env::temp_dir().join(format!(
                "identity_phase09_keys_{}.json",
                uuid7::new_uuid().simple()
            )),
        };
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&artifact.path)?;
        let material = json!({"keys":[
            {"kid":"phase09-a","privateKeyPem":KEY_A_PEM,"n":KEY_A_N,"e":"AQAB"},
            {"kid":"phase09-b","privateKeyPem":KEY_B_PEM,"n":KEY_B_N,"e":"AQAB"}
        ]});
        file.write_all(material.to_string().as_bytes())?;
        drop(file);
        Ok(artifact)
    }
}

impl Drop for KeyFile {
    fn drop(&mut self) {
        let owned = self.path.parent() == Some(std::env::temp_dir().as_path())
            && self
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("identity_phase09_keys_"))
                .and_then(|name| name.strip_suffix(".json"))
                .is_some_and(|suffix| {
                    suffix.len() == 32
                        && suffix.bytes().all(|ch| ch.is_ascii_hexdigit())
                        && Uuid::parse_str(suffix).is_ok_and(uuid7::is_uuid_v7)
                });
        if owned {
            let removed = std::fs::remove_file(&self.path).is_ok();
            eprintln!("identity_phase09_keyset_cleanup removed={removed}");
        }
    }
}

#[derive(Clone)]
struct Client {
    id: Uuid,
    secret: String,
    redirect: String,
    name: String,
}

struct Fixture {
    state: AppState,
    admin: String,
    memberless: Account,
    administrator: Account,
    employee: Client,
    customer: Client,
    _keys: KeyFile,
}

impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        Self::with_token_ttl(pool, 300).await
    }

    async fn with_token_ttl(pool: &PgPool, token_ttl: i64) -> Result<Self> {
        let keys = KeyFile::new()?;
        let tenant = uuid7::new_uuid();
        let org = uuid7::new_uuid();
        let application = uuid7::new_uuid();
        let menu = uuid7::new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned Phase 9 tenant','admin-tenant',true)")
            .bind(tenant).execute(pool).await?;
        sqlx::query(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned Phase 9 root')",
        )
        .bind(org)
        .bind(tenant)
        .execute(pool)
        .await?;
        let administrator = account(pool, "phase09-administrator", true).await?;
        let memberless = account(pool, "phase09-memberless", false).await?;
        let member = uuid7::new_uuid();
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,'EMPLOYEE','ENABLED',1)")
            .bind(member).bind(administrator.id).bind(org).execute(pool).await?;
        sqlx::query("INSERT INTO applications(id,name,code,allowed_member_types) VALUES($1,'Owned Phase 9 management','phase09-management',ARRAY['EMPLOYEE'])")
            .bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(application)
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
        )
        .bind(org)
        .bind(application)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,path) VALUES($1,$2,'MENU','Owned Phase 9 management','system:oauth','/system/oauth/tokens')")
            .bind(menu).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
            .bind(org)
            .bind(menu)
            .execute(pool)
            .await?;
        let settings = Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/identity-phase09".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
            (
                "PUBLIC_APP_URL".into(),
                "https://console.example.invalid".into(),
            ),
            ("SSO_ENABLED".into(), "true".into()),
            ("SSO_ORIGIN".into(), ORIGIN.into()),
            ("OIDC_ENABLED".into(), "true".into()),
            ("OIDC_ISSUER".into(), ORIGIN.into()),
            (
                "OIDC_KEYSET_PATH".into(),
                keys.path
                    .to_str()
                    .ok_or_else(|| anyhow!("owned test keyset path is not UTF-8"))?
                    .into(),
            ),
            ("OIDC_TOKEN_TTL_SECONDS".into(), token_ttl.to_string()),
            ("LOGIN_SOURCE_REQUESTS".into(), "1000".into()),
            ("LOGIN_USER_FAILURES".into(), "1000".into()),
        ]))?;
        let state = AppState::new(pool.clone(), Arc::new(settings));
        let app = router(state.clone());
        let login = call(
            &app,
            Method::POST,
            "/api/auth/login",
            None,
            json!({"username":administrator.username,"password":PASSWORD}),
        )
        .await?;
        expect(
            &login,
            StatusCode::OK,
            "real platform administrator password login",
        )?;
        let context = call(
            &app,
            Method::POST,
            "/api/auth/context",
            Some(text(&login.body, "ticket")?),
            json!({"membershipId":member,"organizationId":org}),
        )
        .await?;
        expect(
            &context,
            StatusCode::OK,
            "real platform employee management context",
        )?;
        let admin = text(&context.body, "accessToken")?.to_owned();
        for (path, kid) in [
            ("/api/oidc/keys/publish", "phase09-a"),
            ("/api/oidc/keys/activate", "phase09-a"),
            ("/api/oidc/keys/publish", "phase09-b"),
        ] {
            let reply = call(&app, Method::POST, path, Some(&admin), json!({"kid":kid})).await?;
            ensure!(
                reply.status.is_success(),
                "owned OIDC signing-key setup did not succeed"
            );
        }
        let employee =
            new_client(&app, &admin, "PLATFORM_CONSOLE", &["EMPLOYEE"], REDIRECT).await?;
        let customer = new_client(
            &app,
            &admin,
            "CUSTOMER_PORTAL",
            &["CUSTOMER"],
            CUSTOMER_REDIRECT,
        )
        .await?;
        Ok(Self {
            state,
            admin,
            memberless,
            administrator,
            employee,
            customer,
            _keys: keys,
        })
    }

    fn app(&self) -> Router {
        router(self.state.clone())
    }
}

async fn new_client(
    app: &Router,
    admin: &str,
    purpose: &str,
    types: &[&str],
    redirect: &str,
) -> Result<Client> {
    let name = format!("Owned Phase 9 {purpose}");
    let reply=call(app,Method::POST,"/api/oidc/clients",Some(admin),json!({
        "name":name,"purpose":purpose,"allowedMemberTypes":types,"redirectUris":[redirect],
        "postLogoutRedirectUris":["https://console.example.invalid/signed-out"],"refreshEnabled":true,
    })).await?;
    ensure!(
        reply.status.is_success(),
        "real OIDC client registration failed"
    );
    let id = Uuid::parse_str(text(&reply.body, "id")?)?;
    ensure!(
        uuid7::is_uuid_v7(id)
            && reply.body["clientId"] == json!(id)
            && reply.body["accessPolicyVersion"] == 1
            && reply.body["purpose"] == purpose
            && reply.body["allowedMemberTypes"] == json!(types),
        "OIDC registration lost its explicit stable identity or RP purpose policy"
    );
    Ok(Client {
        id,
        secret: text(&reply.body, "clientSecret")?.into(),
        redirect: redirect.into(),
        name,
    })
}

fn encoded(fields: &[(&str, String)]) -> String {
    fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encoding::utf8_percent_encode(key, percent_encoding::NON_ALPHANUMERIC),
                percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

struct Flow {
    id: Uuid,
    cookie: String,
    csrf: String,
    state: String,
    nonce: String,
}

fn authorize_parameters(client: &Client, state: &str, nonce: &str) -> Vec<(&'static str, String)> {
    vec![
        ("client_id", client.id.to_string()),
        ("redirect_uri", client.redirect.clone()),
        ("response_type", "code".into()),
        ("scope", "openid profile offline_access".into()),
        ("state", state.into()),
        ("nonce", nonce.into()),
        ("code_challenge", hash::b64url_sha256(VERIFIER)),
        ("code_challenge_method", "S256".into()),
    ]
}

async fn start_flow(
    app: &Router,
    client: &Client,
    sso: Option<&Sso>,
    existing_cookie: Option<&str>,
) -> Result<Flow> {
    start_flow_with(app, client, sso, existing_cookie, &[]).await
}

async fn start_flow_with(
    app: &Router,
    client: &Client,
    sso: Option<&Sso>,
    existing_cookie: Option<&str>,
    extra: &[(&'static str, String)],
) -> Result<Flow> {
    let state = uuid7::new_uuid().simple().to_string();
    let nonce = uuid7::new_uuid().simple().to_string();
    let mut parameters = authorize_parameters(client, &state, &nonce);
    for (name, value) in extra {
        if let Some(parameter) = parameters.iter_mut().find(|(key, _)| key == name) {
            parameter.1 = value.clone();
        } else {
            parameters.push((*name, value.clone()));
        }
    }
    let cookies = [sso.map(|sso| sso.cookie.as_str()), existing_cookie]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("; ");
    let mut headers = vec![("origin", ORIGIN)];
    if !cookies.is_empty() {
        headers.push(("cookie", cookies.as_str()));
    }
    let reply = exchange(
        app,
        Method::GET,
        &format!("/oidc/authorize?{}", encoded(&parameters)),
        None,
        String::new(),
        &headers,
    )
    .await?;
    expect(
        &reply,
        StatusCode::SEE_OTHER,
        "standard OIDC authorization starts a browser-bound interaction",
    )?;
    no_store(&reply)?;
    let location = reply
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("OIDC authorization omitted interaction location"))?;
    let location = reqwest::Url::parse(ORIGIN)?.join(location)?;
    ensure!(
        location.origin().ascii_serialization() == ORIGIN && location.path() == "/sso/login",
        "OIDC authorization redirected to an untrusted interaction origin or path"
    );
    let flow = location
        .query_pairs()
        .find(|(key, _)| key == "flow")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| anyhow!("OIDC authorization omitted its interaction ID"))?;
    let id = Uuid::parse_str(&flow)?;
    ensure!(
        uuid7::is_uuid_v7(id),
        "OIDC interaction does not use a v7 identity"
    );
    let cookie = if let Some(existing) = existing_cookie {
        if reply.headers.contains_key(header::SET_COOKIE) {
            ensure!(
                response_cookie(&reply, BROWSER_COOKIE)? == existing,
                "opening another flow rotated the shared browser binding"
            );
        }
        existing.into()
    } else {
        response_cookie(&reply, BROWSER_COOKIE)?
    };
    cookie_secret(&cookie)?;
    Ok(Flow {
        id,
        cookie,
        csrf: String::new(),
        state,
        nonce,
    })
}

fn flow_cookies(flow: &Flow, sso: Option<&Sso>) -> String {
    if let Some(sso) = sso {
        format!("{}; {}", flow.cookie, sso.cookie)
    } else {
        flow.cookie.clone()
    }
}

async fn interaction(app: &Router, flow: &Flow, sso: Option<&Sso>) -> Result<Reply> {
    browser(
        app,
        Method::GET,
        &format!("/oidc/interaction/{}", flow.id),
        Some(&flow_cookies(flow, sso)),
        None,
        Value::Null,
    )
    .await
}

async fn approve(app: &Router, client: &Client, flow: &mut Flow, sso: &Sso) -> Result<String> {
    let page = interaction(app, flow, Some(sso)).await?;
    expect(
        &page,
        StatusCode::OK,
        "authenticated OIDC interaction state",
    )?;
    ensure!(
        page.body["status"] == "CONSENT_REQUIRED",
        "OIDC flow did not retain its original valid global authentication"
    );
    flow.csrf = text(&page.body, "csrfToken")?.into();
    let approved = browser(
        app,
        Method::POST,
        &format!("/oidc/interaction/{}", flow.id),
        Some(&flow_cookies(flow, Some(sso))),
        Some(&flow.csrf),
        json!({"action":"approve"}),
    )
    .await?;
    expect(
        &approved,
        StatusCode::OK,
        "real CSRF-protected OIDC approval",
    )?;
    let redirect = reqwest::Url::parse(text(&approved.body, "redirectTo")?)?;
    let mut base = redirect.clone();
    base.set_query(None);
    base.set_fragment(None);
    ensure!(
        base.as_str() == client.redirect && redirect.fragment().is_none(),
        "OIDC consent used a different callback target"
    );
    let parameters = redirect.query_pairs().collect::<Vec<_>>();
    ensure!(
        parameters.iter().filter(|(key, _)| key == "state").count() == 1
            && parameters
                .iter()
                .any(|(key, value)| key == "state" && value.as_ref() == flow.state.as_str())
            && parameters.iter().filter(|(key, _)| key == "iss").count() == 1
            && parameters
                .iter()
                .any(|(key, value)| key == "iss" && value == ORIGIN),
        "OIDC callback lost or duplicated the original state or issuer"
    );
    ensure!(
        parameters.iter().filter(|(key, _)| key == "code").count() == 1,
        "OIDC callback omitted or repeated its code"
    );
    parameters
        .into_iter()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| anyhow!("OIDC approval omitted its code"))
}

async fn approved_code(app: &Router, client: &Client, sso: &Sso) -> Result<(Flow, String)> {
    let mut flow = start_flow(app, client, Some(sso), None).await?;
    let code = approve(app, client, &mut flow, sso).await?;
    Ok((flow, code))
}

async fn basic_form(
    app: &Router,
    path: &str,
    client: &Client,
    fields: &[(&str, String)],
) -> Result<Reply> {
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", client.id, client.secret))
    );
    let reply = exchange(
        app,
        Method::POST,
        path,
        None,
        encoded(fields),
        &[
            ("content-type", "application/x-www-form-urlencoded"),
            ("authorization", &basic),
        ],
    )
    .await?;
    no_store(&reply)?;
    Ok(reply)
}

fn code_parameters(client: &Client, code: &str) -> Vec<(&'static str, String)> {
    vec![
        ("grant_type", "authorization_code".into()),
        ("code", code.into()),
        ("redirect_uri", client.redirect.clone()),
        ("code_verifier", VERIFIER.into()),
    ]
}

async fn exchange_code(app: &Router, client: &Client, code: &str) -> Result<Reply> {
    basic_form(app, "/oidc/token", client, &code_parameters(client, code)).await
}

async fn refresh(app: &Router, client: &Client, value: &str) -> Result<Reply> {
    basic_form(
        app,
        "/oidc/token",
        client,
        &[
            ("grant_type", "refresh_token".into()),
            ("refresh_token", value.into()),
        ],
    )
    .await
}

struct Tokens {
    access: String,
    refresh: String,
    id: String,
}

fn tokens(reply: &Reply) -> Result<Tokens> {
    expect(reply, StatusCode::OK, "standard OIDC token exchange")?;
    no_store(reply)?;
    ensure!(
        reply.body["token_type"] == "Bearer"
            && reply.body["expires_in"].as_i64().is_some_and(|ttl| ttl > 0),
        "OIDC token response lost its standard type or lifetime"
    );
    Ok(Tokens {
        access: text(&reply.body, "access_token")?.into(),
        refresh: text(&reply.body, "refresh_token")?.into(),
        id: text(&reply.body, "id_token")?.into(),
    })
}

async fn userinfo(app: &Router, token: &str) -> Result<Reply> {
    call(app, Method::GET, "/oidc/userinfo", Some(token), Value::Null).await
}

fn oauth_error(reply: &Reply, status: StatusCode, expected: &str) -> Result<()> {
    expect(reply, status, "OIDC negative protocol response")?;
    ensure!(
        reply.body["error"] == expected,
        "OIDC rejection must retain its exact {expected} protocol category"
    );
    ensure!(
        reply.body.get("access_token").is_none()
            && reply.body.get("id_token").is_none()
            && reply.body.get("refresh_token").is_none(),
        "OIDC rejection returned a partial credential"
    );
    Ok(())
}

async fn jwks(app: &Router) -> Result<Value> {
    let reply = call(app, Method::GET, "/oidc/jwks", None, Value::Null).await?;
    expect(&reply, StatusCode::OK, "public OIDC signing keys")?;
    ensure!(
        reply.body["keys"].as_array().is_some(),
        "OIDC JWKS omitted its key array"
    );
    Ok(reply.body)
}

fn verify_id(token: &str, keys: &Value, client: Uuid, nonce: &str, subject: Uuid) -> Result<Value> {
    let header = jsonwebtoken::decode_header(token)
        .map_err(|_| anyhow!("independent OIDC verifier rejected token header"))?;
    ensure!(
        header.alg == jsonwebtoken::Algorithm::RS256,
        "independent OIDC verifier rejected algorithm downgrade"
    );
    let kid = header
        .kid
        .as_deref()
        .ok_or_else(|| anyhow!("ID Token omitted kid"))?;
    let matching = keys["keys"]
        .as_array()
        .ok_or_else(|| anyhow!("JWKS missing keys"))?
        .iter()
        .filter(|key| key["kid"].as_str() == Some(kid))
        .collect::<Vec<_>>();
    ensure!(
        matching.len() == 1,
        "independent OIDC verifier rejected an absent or ambiguous signing key"
    );
    let key = matching[0];
    ensure!(
        key["kty"] == "RSA" && key["alg"] == "RS256" && key["use"] == "sig",
        "OIDC JWKS key lost its explicit RSA signing algorithm or use"
    );
    let decoding = jsonwebtoken::DecodingKey::from_rsa_components(text(key, "n")?, text(key, "e")?)
        .map_err(|_| anyhow!("independent OIDC verifier rejected RSA public components"))?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.leeway = 0;
    validation.set_issuer(&[ORIGIN]);
    validation.set_audience(&[client.to_string()]);
    validation.set_required_spec_claims(&["iss", "sub", "aud", "iat", "exp"]);
    let claims = jsonwebtoken::decode::<Value>(token, &decoding, &validation)
        .map_err(|_| anyhow!("independent OIDC verifier rejected signed claims"))?
        .claims;
    ensure!(
        claims["sub"] == json!(subject)
            && uuid7::is_uuid_v7(subject)
            && claims["nonce"].as_str() == Some(nonce)
            && claims["auth_time"].as_i64().is_some_and(|time| time > 0)
            && claims["iat"]
                .as_i64()
                .zip(claims["exp"].as_i64())
                .is_some_and(|(iat, exp)| iat <= chrono::Utc::now().timestamp() + 1 && exp > iat),
        "independent OIDC verifier rejected stable subject, nonce or authentication time"
    );
    for forbidden in [
        "tenant",
        "tenantId",
        "tenant_id",
        "org",
        "organizationId",
        "organization_id",
        "membershipId",
        "membership_id",
        "memberType",
        "member_type",
        "identityVersion",
        "identity_version",
        "roles",
        "permissions",
        "isSuperAdmin",
        "is_super_admin",
        "passwordHash",
        "password_hash",
    ] {
        ensure!(
            claims.get(forbidden).is_none(),
            "OIDC ID Token leaked platform authority field {forbidden}"
        );
    }
    Ok(claims)
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE09_TEST_DATABASE_URL and disposable database creation"]
async fn phase09_memberless_standard_code_s256_and_independent_id_token_validation() -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let discovered=exchange(&app,Method::GET,"/.well-known/openid-configuration",None,String::new(),&[("host","untrusted.example.invalid")]).await?;
        expect(&discovered,StatusCode::OK,"standard OIDC discovery")?;
        ensure!(discovered.body["issuer"]==ORIGIN
            && discovered.body["authorization_endpoint"]==format!("{ORIGIN}/oidc/authorize")
            && discovered.body["token_endpoint"]==format!("{ORIGIN}/oidc/token")
            && discovered.body["userinfo_endpoint"]==format!("{ORIGIN}/oidc/userinfo")
            && discovered.body["jwks_uri"]==format!("{ORIGIN}/oidc/jwks")
            && discovered.body["response_types_supported"]==json!(["code"])
            && discovered.body["id_token_signing_alg_values_supported"]==json!(["RS256"])
            && discovered.body["code_challenge_methods_supported"]==json!(["S256"])
            && discovered.body["token_endpoint_auth_methods_supported"]==json!(["client_secret_basic"]),
            "discovery changed the configured issuer or standard supported protocol");
        let legacy=call(&app,Method::GET,"/oauth/.well-known/openid-configuration",None,Value::Null).await?;
        expect(&legacy,StatusCode::OK,"preexisting legacy OAuth discovery remains available")?;
        ensure!(legacy.body["issuer"]=="https://console.example.invalid"
            && legacy.body["token_endpoint"]=="https://console.example.invalid/oauth/token",
            "new provider replaced the existing OAuth endpoint contract");
        let mut flow=start_flow(&app,&fixture.employee,None,None).await?;
        let unauthenticated=interaction(&app,&flow,None).await?;
        expect(&unauthenticated,StatusCode::OK,"browser-bound unauthenticated interaction")?;
        ensure!(unauthenticated.body["status"]=="LOGIN_REQUIRED","new unauthenticated flow inferred an account or membership");
        let sso=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let code=approve(&app,&fixture.employee,&mut flow,&sso).await?;
        let pair=tokens(&exchange_code(&app,&fixture.employee,&code).await?)?;
        let keys=jwks(&app).await?;
        let claims=verify_id(&pair.id,&keys,fixture.employee.id,&flow.nonce,fixture.memberless.id)?;
        let (sid,auth_time):(Uuid,chrono::DateTime<chrono::Utc>)=sqlx::query_as("SELECT id,authenticated_at FROM sso_sessions WHERE token_hash=$1")
            .bind(hash::sha256_hex(cookie_secret(&sso.cookie)?)).fetch_one(&database.pool).await?;
        ensure!(claims["sid"]==json!(sid) && claims["auth_time"]==auth_time.timestamp(),
            "ID Token did not preserve its exact original global session and authentication time");
        let info=userinfo(&app,&pair.access).await?;
        expect(&info,StatusCode::OK,"opaque access token reaches provider userinfo")?;
        ensure!(info.body["sub"]==json!(fixture.memberless.id),"provider userinfo changed the stable memberless subject");
        for path in ["/api/auth/profile","/api/customer/profile","/oauth/userinfo","/oidc/userinfo"] {
            expect(&call(&app,Method::GET,path,Some(&pair.id),Value::Null).await?,StatusCode::UNAUTHORIZED,
                "ID Token cannot substitute for a platform, legacy OAuth or provider access credential")?;
        }
        expect(&call(&app,Method::GET,"/api/auth/profile",Some(&pair.access),Value::Null).await?,StatusCode::UNAUTHORIZED,
            "provider account access does not grant platform context")?;
        ensure!(verify_id(&pair.id,&keys,fixture.customer.id,&flow.nonce,fixture.memberless.id).is_err()
            && verify_id(&pair.id,&keys,fixture.employee.id,"different-expected-nonce",fixture.memberless.id).is_err(),
            "independent client verifier accepted a wrong audience or expected nonce");
        let signing=jsonwebtoken::EncodingKey::from_rsa_pem(KEY_A_PEM.as_bytes())
            .map_err(|_|anyhow!("test-only RSA signing fixture is invalid"))?;
        for mutation in ["iss","aud","nonce","v4-sub","other-sub","exp"] {
            let mut altered=claims.clone();
            match mutation {
                "iss"=>altered["iss"]=json!("https://untrusted.example.invalid"),
                "aud"=>altered["aud"]=json!(fixture.customer.id),
                "nonce"=>altered["nonce"]=json!("incorrect-signed-nonce"),
                "v4-sub"=>altered["sub"]=json!(Uuid::new_v4()),
                "other-sub"=>altered["sub"]=json!(fixture.administrator.id),
                "exp"=>altered["exp"]=json!(chrono::Utc::now().timestamp()-1),
                _=>return Err(anyhow!("unknown owned ID Token validation mutation")),
            }
            let mut header=jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);header.kid=Some("phase09-a".into());
            let altered=jsonwebtoken::encode(&header,&altered,&signing).map_err(|_|anyhow!("test-only altered claims could not be signed"))?;
            ensure!(verify_id(&altered,&keys,fixture.employee.id,&flow.nonce,fixture.memberless.id).is_err(),
                "independent OIDC verifier accepted altered {mutation}");
        }
        let mut downgrade_header=jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);downgrade_header.kid=Some("phase09-a".into());
        let downgraded=jsonwebtoken::encode(&downgrade_header,&claims,&jsonwebtoken::EncodingKey::from_secret(SECRET.as_bytes()))?;
        ensure!(verify_id(&downgraded,&keys,fixture.employee.id,&flow.nonce,fixture.memberless.id).is_err(),"independent verifier accepted symmetric algorithm downgrade");
        let unsigned=format!("{}.{}.",base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none","kid":"phase09-a"}"#),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string()));
        ensure!(verify_id(&unsigned,&keys,fixture.employee.id,&flow.nonce,fixture.memberless.id).is_err(),"independent verifier accepted an unsigned ID Token");
        let administrator=sso_login(&app,&fixture.administrator,PASSWORD).await?;
        let (admin_flow,admin_code)=approved_code(&app,&fixture.employee,&administrator).await?;
        let admin_pair=tokens(&exchange_code(&app,&fixture.employee,&admin_code).await?)?;
        verify_id(&admin_pair.id,&keys,fixture.employee.id,&admin_flow.nonce,fixture.administrator.id)?;
        let memberships:i64=sqlx::query_scalar("SELECT count(*) FROM memberships WHERE user_id=$1")
            .bind(fixture.memberless.id).fetch_one(&database.pool).await?;
        ensure!(memberships==0,"OIDC code exchange manufactured an organization context for a memberless account");
        let audit:String=sqlx::query_scalar("SELECT COALESCE(string_agg(to_jsonb(a)::text,''),'') FROM audit_logs a")
            .fetch_one(&database.pool).await?;
        for sensitive in [pair.access.as_str(),pair.refresh.as_str(),pair.id.as_str(),code.as_str(),flow.nonce.as_str(),flow.state.as_str(),fixture.employee.secret.as_str(),cookie_secret(&sso.cookie)?] {
            ensure!(!audit.contains(sensitive),"OIDC audit persisted a token, code, nonce, state, secret or browser credential");
        }
        Ok(())
    }.await;
    database.finish(assertions).await
}

fn authorization_error(reply: &Reply, client: &Client, state: &str, expected: &str) -> Result<()> {
    expect(
        reply,
        StatusCode::SEE_OTHER,
        "trusted OIDC authorization error callback",
    )?;
    let location = reply
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("OIDC authorization error omitted registered callback"))?;
    let url = reqwest::Url::parse(location)?;
    let mut base = url.clone();
    base.set_query(None);
    base.set_fragment(None);
    ensure!(
        base.as_str() == client.redirect
            && url.fragment().is_none()
            && url
                .query_pairs()
                .any(|(key, value)| key == "state" && value == state)
            && url
                .query_pairs()
                .any(|(key, value)| key == "iss" && value == ORIGIN)
            && url.query_pairs().filter(|(key, _)| key == "error").count() == 1
            && url
                .query_pairs()
                .any(|(key, value)| key == "error" && value == expected)
            && !url.query_pairs().any(|(key, _)| key == "code"),
        "OIDC authorization error lost its exact callback, state, issuer or {expected} category"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE09_TEST_DATABASE_URL and disposable database creation"]
async fn phase09_authorization_and_token_negative_protocols_preserve_one_time_code_binding(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let sso=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let base=authorize_parameters(&fixture.employee,"owned-negative-state","owned-negative-nonce");
        for (name,value,redirect_error) in [
            ("redirect_uri","https://attacker.example.invalid/callback",None),
            ("redirect_uri","https://console.example.invalid/*",None),
            ("client_id","00000000-0000-4000-8000-000000000001",None),
            ("response_type","token",Some("unsupported_response_type")),
            ("code_challenge_method","plain",Some("invalid_request")),
            ("code_challenge_method","",Some("invalid_request")),
            ("code_challenge","not-a-canonical-S256-challenge",Some("invalid_request")),
            ("nonce","",Some("invalid_request")),
            ("scope","profile",Some("invalid_scope")),
            ("scope","openid administrator",Some("invalid_scope")),
            ("prompt","none login",Some("invalid_request")),
            ("max_age","-1",Some("invalid_request")),
        ] {
            let mut fields=base.clone();
            if let Some(field)=fields.iter_mut().find(|(key,_)|*key==name) {field.1=value.into();}
            else {fields.push((name,value.into()));}
            let reply=exchange(&app,Method::GET,&format!("/oidc/authorize?{}",encoded(&fields)),None,String::new(),&[("cookie",&sso.cookie)]).await?;
            no_store(&reply)?;
            if let Some(expected)=redirect_error {authorization_error(&reply,&fixture.employee,"owned-negative-state",expected)?;}
            else {
                oauth_error(&reply,StatusCode::BAD_REQUEST,"invalid_request")?;
                ensure!(!reply.headers.contains_key(header::LOCATION),"invalid client or callback caused an attacker-controlled redirect");
            }
        }
        let mut repeated=base.clone();repeated.push(("redirect_uri",fixture.employee.redirect.clone()));
        oauth_error(&exchange(&app,Method::GET,&format!("/oidc/authorize?{}",encoded(&repeated)),None,String::new(),&[]).await?,
            StatusCode::BAD_REQUEST,"invalid_request")?;
        let unknown=uuid7::new_uuid().to_string();
        let mut fields=base.clone();fields.iter_mut().find(|(key,_)|*key=="client_id").ok_or_else(||anyhow!("owned authorization parameters omitted client ID"))?.1=unknown;
        oauth_error(&exchange(&app,Method::GET,&format!("/oidc/authorize?{}",encoded(&fields)),None,String::new(),&[]).await?,
            StatusCode::BAD_REQUEST,"invalid_request")?;
        let (flow,code)=approved_code(&app,&fixture.employee,&sso).await?;
        let original=code_parameters(&fixture.employee,&code);
        for (name,value,expected) in [
            ("redirect_uri",CUSTOMER_REDIRECT,"invalid_grant"),
            ("code_verifier","Another-valid-code-verifier-0123456789012345678901234567890123456789","invalid_grant"),
            ("code_verifier","short","invalid_grant"),
        ] {
            let mut fields=original.clone();
            fields.iter_mut().find(|(key,_)|*key==name).ok_or_else(||anyhow!("owned token parameters omitted binding field"))?.1=value.into();
            oauth_error(&basic_form(&app,"/oidc/token",&fixture.employee,&fields).await?,StatusCode::BAD_REQUEST,expected)?;
        }
        let mut missing=original.clone();missing.retain(|(key,_)|*key!="code_verifier");
        oauth_error(&basic_form(&app,"/oidc/token",&fixture.employee,&missing).await?,StatusCode::BAD_REQUEST,"invalid_request")?;
        oauth_error(&exchange_code(&app,&fixture.customer,&code).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        let mut wrong_secret=fixture.employee.clone();wrong_secret.secret="0".repeat(64);
        oauth_error(&exchange_code(&app,&wrong_secret,&code).await?,StatusCode::UNAUTHORIZED,"invalid_client")?;
        oauth_error(&form(&app,"/oidc/token",&original).await?,StatusCode::UNAUTHORIZED,"invalid_client")?;
        let mut mixed=original.clone();mixed.push(("client_id",fixture.customer.id.to_string()));
        oauth_error(&basic_form(&app,"/oidc/token",&fixture.employee,&mixed).await?,StatusCode::UNAUTHORIZED,"invalid_client")?;
        let mut mixed_secret=original.clone();mixed_secret.push(("client_secret",fixture.employee.secret.clone()));
        oauth_error(&basic_form(&app,"/oidc/token",&fixture.employee,&mixed_secret).await?,StatusCode::UNAUTHORIZED,"invalid_client")?;
        let mut duplicate=original.clone();duplicate.push(("code",code.clone()));
        oauth_error(&basic_form(&app,"/oidc/token",&fixture.employee,&duplicate).await?,StatusCode::BAD_REQUEST,"invalid_request")?;
        oauth_error(&basic_form(&app,"/oidc/token?grant_type=authorization_code",&fixture.employee,&original).await?,StatusCode::BAD_REQUEST,"invalid_request")?;
        let unconsumed:bool=sqlx::query_scalar("SELECT consumed_at IS NULL FROM oidc_authorization_codes WHERE token_hash=$1")
            .bind(hash::sha256_hex(&code)).fetch_one(&database.pool).await?;
        ensure!(unconsumed,"invalid client, callback or PKCE proof consumed the legitimate authorization code");
        let mut redundant_identifier=original.clone();redundant_identifier.push(("client_id",fixture.employee.id.to_string()));
        let pair=tokens(&basic_form(&app,"/oidc/token",&fixture.employee,&redundant_identifier).await?)?;
        verify_id(&pair.id,&jwks(&app).await?,fixture.employee.id,&flow.nonce,fixture.memberless.id)?;
        let posted=request(&app,Method::POST,"/oidc/userinfo",Some(&pair.access),"application/x-www-form-urlencoded",String::new()).await?;
        expect(&posted,StatusCode::OK,"standard provider userinfo supports authenticated POST")?;
        oauth_error(&exchange_code(&app,&fixture.employee,&code).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        expect(&userinfo(&app,&pair.access).await?,StatusCode::UNAUTHORIZED,"proved code replay also retires the already-issued grant family")?;
        let (_,expired)=approved_code(&app,&fixture.employee,&sso).await?;
        sqlx::query("UPDATE oidc_authorization_codes SET created_at=clock_timestamp()-interval '1 hour',expires_at=clock_timestamp()-interval '1 second' WHERE token_hash=$1")
            .bind(hash::sha256_hex(&expired)).execute(&database.pool).await?;
        oauth_error(&exchange_code(&app,&fixture.employee,&expired).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        let (_,first_code)=approved_code(&app,&fixture.employee,&sso).await?;
        let (_,second_code)=approved_code(&app,&fixture.employee,&sso).await?;
        let mut limited_config=(*fixture.state.config).clone();
        limited_config.oauth_introspection_attempts_per_minute=1;
        let limited=router(AppState::new(database.pool.clone(),Arc::new(limited_config)));
        let basic=format!("Basic {}",base64::engine::general_purpose::STANDARD.encode(format!("{}:{}",fixture.employee.id,fixture.employee.secret)));
        let headers=[("content-type","application/x-www-form-urlencoded"),("authorization",basic.as_str())];
        let peer=SocketAddr::from(([192,0,2,199],39009));
        let positive=exchange_from(&limited,Method::POST,"/oidc/token",None,
            encoded(&code_parameters(&fixture.employee,&first_code)),&headers,peer).await?;
        let positive=tokens(&positive)?;
        let rate_limited=exchange_from(&limited,Method::POST,"/oidc/token",None,
            encoded(&code_parameters(&fixture.employee,&second_code)),&headers,peer).await?;
        oauth_error(&rate_limited,StatusCode::TOO_MANY_REQUESTS,"temporarily_unavailable")?;
        no_store(&rate_limited)?;
        ensure!(rate_limited.body.get("code").is_none() && rate_limited.headers.get(header::RETRY_AFTER)
            .and_then(|value|value.to_str().ok()).and_then(|value|value.parse::<u64>().ok()).is_some_and(|value|value>0),
            "OIDC throttling used business JSON or omitted a positive Retry-After");
        let unconsumed:bool=sqlx::query_scalar("SELECT consumed_at IS NULL FROM oidc_authorization_codes WHERE token_hash=$1")
            .bind(hash::sha256_hex(&second_code)).fetch_one(&database.pool).await?;
        ensure!(unconsumed,"rate-limited OIDC request partially consumed its legitimate code");
        expect(&userinfo(&app,&positive.access).await?,StatusCode::OK,"token rate admission does not revoke the preceding successful grant")?;
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE09_TEST_DATABASE_URL and disposable database creation"]
async fn phase09_refresh_rotation_reuse_revokes_only_its_grant_family() -> Result<()> {
    let database = Database::create().await?;
    let assertions = async {
        let fixture = Fixture::create(&database.pool).await?;
        let app = fixture.app();
        let sso = sso_login(&app, &fixture.memberless, PASSWORD).await?;
        let (flow, code) = approved_code(&app, &fixture.employee, &sso).await?;
        let first = tokens(&exchange_code(&app, &fixture.employee, &code).await?)?;
        let (other_flow, other_code) = approved_code(&app, &fixture.customer, &sso).await?;
        let other = tokens(&exchange_code(&app, &fixture.customer, &other_code).await?)?;
        let keys = jwks(&app).await?;
        let original = verify_id(
            &first.id,
            &keys,
            fixture.employee.id,
            &flow.nonce,
            fixture.memberless.id,
        )?;
        oauth_error(
            &refresh(&app, &fixture.customer, &first.refresh).await?,
            StatusCode::BAD_REQUEST,
            "invalid_grant",
        )?;
        expect(
            &userinfo(&app, &first.access).await?,
            StatusCode::OK,
            "wrong-client refresh attempt cannot retire the legitimate family",
        )?;
        let rotated = tokens(&refresh(&app, &fixture.employee, &first.refresh).await?)?;
        ensure!(
            rotated.refresh != first.refresh && rotated.access != first.access,
            "refresh proof did not rotate its opaque credentials"
        );
        let refreshed = verify_id(
            &rotated.id,
            &keys,
            fixture.employee.id,
            &flow.nonce,
            fixture.memberless.id,
        )?;
        ensure!(
            refreshed["auth_time"] == original["auth_time"],
            "refresh silently refreshed the original authentication time"
        );
        oauth_error(
            &refresh(&app, &fixture.employee, &first.refresh).await?,
            StatusCode::BAD_REQUEST,
            "invalid_grant",
        )?;
        oauth_error(
            &refresh(&app, &fixture.employee, &rotated.refresh).await?,
            StatusCode::BAD_REQUEST,
            "invalid_grant",
        )?;
        for access in [&first.access, &rotated.access] {
            expect(
                &userinfo(&app, access).await?,
                StatusCode::UNAUTHORIZED,
                "refresh reuse retires the entire descendant access family",
            )?;
        }
        expect(
            &sso_session(&app, &sso).await?,
            StatusCode::OK,
            "refresh reuse preserves global account authentication",
        )?;
        expect(
            &userinfo(&app, &other.access).await?,
            StatusCode::OK,
            "refresh reuse preserves another registered client's grant",
        )?;
        verify_id(
            &other.id,
            &keys,
            fixture.customer.id,
            &other_flow.nonce,
            fixture.memberless.id,
        )?;
        let other_rotated = tokens(&refresh(&app, &fixture.customer, &other.refresh).await?)?;
        expect(
            &userinfo(&app, &other_rotated.access).await?,
            StatusCode::OK,
            "another client's refresh remains usable after family reuse detection",
        )?;
        let (_, concurrent_code) = approved_code(&app, &fixture.employee, &sso).await?;
        let concurrent = tokens(&exchange_code(&app, &fixture.employee, &concurrent_code).await?)?;
        let (left, right) = tokio::join!(
            refresh(&app, &fixture.employee, &concurrent.refresh),
            refresh(&app, &fixture.employee, &concurrent.refresh)
        );
        let left = left?;
        let right = right?;
        let (winner, loser) = if left.status == StatusCode::OK {
            (&left, &right)
        } else {
            (&right, &left)
        };
        let winner = tokens(winner)?;
        oauth_error(loser, StatusCode::BAD_REQUEST, "invalid_grant")?;
        expect(
            &userinfo(&app, &winner.access).await?,
            StatusCode::UNAUTHORIZED,
            "concurrent reuse detection also retires the winner's just-issued access",
        )?;
        oauth_error(
            &refresh(&app, &fixture.employee, &winner.refresh).await?,
            StatusCode::BAD_REQUEST,
            "invalid_grant",
        )?;
        expect(
            &sso_session(&app, &sso).await?,
            StatusCode::OK,
            "concurrent refresh reuse still preserves the global session",
        )?;
        Ok(())
    }
    .await;
    database.finish(assertions).await
}

fn changed_client(client: &Client, expected: i64) -> Value {
    json!({"name":client.name,"purpose":"CUSTOMER_PORTAL","allowedMemberTypes":["CUSTOMER"],
        "redirectUris":[CUSTOMER_REDIRECT],"postLogoutRedirectUris":["https://customer.example.invalid/signed-out"],
        "refreshEnabled":true,"status":"ENABLED","expectedAccessPolicyVersion":expected})
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE09_TEST_DATABASE_URL and disposable database creation"]
async fn phase09_client_purpose_policy_retires_all_old_grants_without_membership_or_global_revocation(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let single=PgPoolOptions::new().max_connections(1).acquire_timeout(Duration::from_secs(2))
            .connect_with((*database.pool.connect_options()).clone()).await?;
        let management=async {
            let single_app=router(AppState::new(single.clone(),fixture.state.config.clone()));
            let registered=new_client(&single_app,&fixture.admin,"ACCOUNT",&["EMPLOYEE"],REDIRECT).await?;
            expect(&call(&single_app,Method::PATCH,&format!("/api/oidc/clients/{}",registered.id),Some(&fixture.admin),changed_client(&registered,1)).await?,
                StatusCode::OK,"one-connection pool can manage a client without nested authority acquisition")?;
            expect(&call(&single_app,Method::POST,"/api/oidc/keys/activate",Some(&fixture.admin),json!({"kid":"phase09-b"})).await?,
                StatusCode::OK,"one-connection pool can activate a published signing key")?;
            expect(&call(&single_app,Method::DELETE,"/api/oidc/keys/phase09-a",Some(&fixture.admin),Value::Null).await?,
                StatusCode::OK,"one-connection pool can retire an unsigned inactive key")?;
            Ok::<_,anyhow::Error>(())
        }.await;
        single.close().await;
        management?;
        let sso=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let admin_sso=sso_login(&app,&fixture.administrator,PASSWORD).await?;
        let (_,first_code)=approved_code(&app,&fixture.employee,&sso).await?;
        let first=tokens(&exchange_code(&app,&fixture.employee,&first_code).await?)?;
        let (_,pending)=approved_code(&app,&fixture.employee,&sso).await?;
        let (_,admin_code)=approved_code(&app,&fixture.employee,&admin_sso).await?;
        let admin=tokens(&exchange_code(&app,&fixture.employee,&admin_code).await?)?;
        let (_,other_code)=approved_code(&app,&fixture.customer,&sso).await?;
        let other=tokens(&exchange_code(&app,&fixture.customer,&other_code).await?)?;
        let updated=call(&app,Method::PATCH,&format!("/api/oidc/clients/{}",fixture.employee.id),Some(&fixture.admin),changed_client(&fixture.employee,1)).await?;
        expect(&updated,StatusCode::OK,"real registered RP purpose and allowed-type policy change")?;
        ensure!(updated.body["accessPolicyVersion"]==2 && updated.body["purpose"]=="CUSTOMER_PORTAL"
            && updated.body["allowedMemberTypes"]==json!(["CUSTOMER"]) && updated.body.get("clientSecret").is_none(),
            "OIDC policy edit lost its generation/purpose or exposed the registration secret");
        let counts:(i64,i64)=sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE revoked_at IS NOT NULL) FROM oidc_grants WHERE client_id=$1")
            .bind(fixture.employee.id).fetch_one(&database.pool).await?;
        ensure!(counts.0>=3 && counts.0==counts.1,"client policy owner failed to retire every account's old grant");
        for pair in [&first,&admin] {
            expect(&userinfo(&app,&pair.access).await?,StatusCode::UNAUTHORIZED,"changed client invalidates all accounts' old access")?;
            oauth_error(&refresh(&app,&fixture.employee,&pair.refresh).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        }
        oauth_error(&exchange_code(&app,&fixture.employee,&pending).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        // Explicit delayed-cleanup/corrupt-row injection: keep the old generation
        // while restoring physical liveness. No trigger or constraint is disabled.
        sqlx::query("UPDATE oidc_grants SET revoked_at=NULL WHERE client_id=$1").bind(fixture.employee.id).execute(&database.pool).await?;
        expect(&userinfo(&app,&first.access).await?,StatusCode::UNAUTHORIZED,"client generation remains authoritative when old physical grant rows are live")?;
        oauth_error(&exchange_code(&app,&fixture.employee,&pending).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        for global in [&sso,&admin_sso] {expect(&sso_session(&app,global).await?,StatusCode::OK,"client policy preserves member-independent global sessions")?;}
        expect(&userinfo(&app,&other.access).await?,StatusCode::OK,"client policy preserves a different registered RP")?;
        let mut replacement=fixture.employee.clone();replacement.redirect=CUSTOMER_REDIRECT.into();
        let (flow,new_code)=approved_code(&app,&replacement,&sso).await?;
        let new_pair=tokens(&exchange_code(&app,&replacement,&new_code).await?)?;
        verify_id(&new_pair.id,&jwks(&app).await?,replacement.id,&flow.nonce,fixture.memberless.id)?;
        let members:i64=sqlx::query_scalar("SELECT count(*) FROM memberships WHERE user_id=$1").bind(fixture.memberless.id).fetch_one(&database.pool).await?;
        ensure!(members==0,"client allowed member types created or inferred an account membership");
        expect(&call(&app,Method::PATCH,&format!("/api/oidc/clients/{}",fixture.employee.id),Some(&fixture.admin),changed_client(&fixture.employee,1)).await?,
            StatusCode::CONFLICT,"stale client policy generation cannot overwrite a current registration")?;
        let listing=call(&app,Method::GET,"/api/oidc/clients",Some(&fixture.admin),Value::Null).await?;
        expect(&listing,StatusCode::OK,"OIDC management list")?;
        let rows=listing.body["records"].as_array().ok_or_else(||anyhow!("OIDC client list omitted records"))?;
        ensure!(rows.iter().all(|row|row.get("clientSecret").is_none()&&row.get("secretHash").is_none()&&row.get("secret_hash").is_none()),
            "OIDC client listing leaked a secret or secret hash");
        for (purpose,types,redirect) in [
            ("CUSTOMER_PORTAL",vec!["EMPLOYEE"],REDIRECT),
            ("PLATFORM_CONSOLE",vec!["CUSTOMER"],REDIRECT),
            ("ACCOUNT",vec!["EMPLOYEE"],"https://console.example.invalid/*"),
            ("ACCOUNT",vec!["EMPLOYEE"],"http://console.example.invalid/callback"),
        ] {
            expect(&call(&app,Method::POST,"/api/oidc/clients",Some(&fixture.admin),json!({"name":"Owned invalid registration","purpose":purpose,
                "allowedMemberTypes":types,"redirectUris":[redirect],"refreshEnabled":true})).await?,StatusCode::BAD_REQUEST,
                "client registration requires exact callbacks and a consistent declared RP purpose")?;
        }
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE09_TEST_DATABASE_URL and disposable database creation"]
async fn phase09_browser_interactions_bind_the_original_sid_and_require_fresh_login_after_restart(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let first=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let switched=sso_login(&app,&fixture.administrator,PASSWORD).await?;
        let mut flow=start_flow(&app,&fixture.employee,Some(&first),None).await?;
        let page=interaction(&app,&flow,Some(&first)).await?;
        expect(&page,StatusCode::OK,"original authenticated interaction")?;
        ensure!(page.body["status"]=="CONSENT_REQUIRED","flow did not capture its original session");
        flow.csrf=text(&page.body,"csrfToken")?.into();
        let second=start_flow(&app,&fixture.customer,Some(&first),Some(&flow.cookie)).await?;
        let second_page=interaction(&app,&second,Some(&first)).await?;
        let second_csrf=text(&second_page.body,"csrfToken")?;
        ensure!(second_csrf!=flow.csrf,"two interactions share one CSRF proof despite different request identities");
        // A cross-site form POST carries the non-authorizing SameSite=None
        // binder while the authentication Cookie remains Lax and is omitted.
        // Returning to a same-origin interaction can then bind the real SSO.
        let post_state="owned-post-authorize-state";let post_nonce="owned-post-authorize-nonce";
        let posted=exchange(&app,Method::POST,"/oidc/authorize",None,encoded(&authorize_parameters(&fixture.employee,post_state,post_nonce)),
            &[("content-type","application/x-www-form-urlencoded"),("cookie",&flow.cookie)]).await?;
        expect(&posted,StatusCode::SEE_OTHER,"standard cross-site POST authorization preserves the independent browser binder")?;
        if posted.headers.contains_key(header::SET_COOKIE) {
            ensure!(response_cookie(&posted,BROWSER_COOKIE)?==flow.cookie,"cross-site POST authorization rotated another tab's browser binding");
        }
        let target=posted.headers.get(header::LOCATION).and_then(|value|value.to_str().ok()).ok_or_else(||anyhow!("POST authorization omitted interaction location"))?;
        let target=reqwest::Url::parse(ORIGIN)?.join(target)?;
        let post_id=target.query_pairs().find(|(key,_)|key=="flow").map(|(_,value)|value.into_owned()).ok_or_else(||anyhow!("POST authorization omitted flow ID"))?;
        let mut post_flow=Flow{id:Uuid::parse_str(&post_id)?,cookie:flow.cookie.clone(),csrf:String::new(),state:post_state.into(),nonce:post_nonce.into()};
        let _post_code=approve(&app,&fixture.employee,&mut post_flow,&first).await?;
        ensure!(interaction(&app,&flow,Some(&first)).await?.body["status"]=="CONSENT_REQUIRED"
            && interaction(&app,&second,Some(&first)).await?.body["status"]=="CONSENT_REQUIRED",
            "POST authorization invalidated another pending tab's immutable interaction");
        expect(&browser(&app,Method::POST,&format!("/oidc/interaction/{}",flow.id),Some(&flow_cookies(&flow,Some(&first))),Some(second_csrf),json!({"action":"approve"})).await?,
            StatusCode::FORBIDDEN,"another interaction's CSRF proof cannot approve this request")?;
        expect(&browser(&app,Method::GET,&format!("/oidc/interaction/{}",flow.id),Some(&first.cookie),None,Value::Null).await?,
            StatusCode::FORBIDDEN,"global SSO does not replace the independent browser flow binding")?;
        let changed=interaction(&app,&flow,Some(&switched)).await?;
        expect(&changed,StatusCode::OK,"different authenticated account receives a restart requirement")?;
        ensure!(changed.body["status"]=="RESTART_REQUIRED" && changed.body["account"].is_null(),
            "flow silently rebound to a different account or disclosed its prior account");
        expect(&browser(&app,Method::POST,&format!("/oidc/interaction/{}",flow.id),Some(&flow_cookies(&flow,Some(&switched))),Some(&flow.csrf),json!({"action":"approve"})).await?,
            StatusCode::UNAUTHORIZED,"switched global session cannot approve an originally bound flow")?;
        let restart=browser(&app,Method::POST,&format!("/oidc/interaction/{}",flow.id),Some(&flow_cookies(&flow,Some(&switched))),Some(&flow.csrf),json!({"action":"restart"})).await?;
        expect(&restart,StatusCode::OK,"explicit immutable OIDC flow restart")?;
        let target=reqwest::Url::parse(ORIGIN)?.join(text(&restart.body,"redirectTo")?)?;
        ensure!(target.path()=="/sso/login" && target.origin().ascii_serialization()==ORIGIN,"restart accepts an untrusted browser return target");
        let new_id=target.query_pairs().find(|(key,_)|key=="flow").map(|(_,value)|value.into_owned())
            .ok_or_else(||anyhow!("restart omitted its replacement flow"))?;
        let mut replacement=Flow{id:Uuid::parse_str(&new_id)?,cookie:flow.cookie.clone(),csrf:String::new(),state:flow.state.clone(),nonce:flow.nonce.clone()};
        ensure!(replacement.id!=flow.id && uuid7::is_uuid_v7(replacement.id),"restart reused the consumed interaction identity");
        let immutable:bool=sqlx::query_scalar("SELECT (a.client_id,a.client_policy_version,a.redirect_uri,a.scope,a.state,a.nonce,a.code_challenge) \
            IS NOT DISTINCT FROM (b.client_id,b.client_policy_version,b.redirect_uri,b.scope,b.state,b.nonce,b.code_challenge) \
            AND a.consumed_at IS NOT NULL AND b.session_id IS NULL AND b.force_login \
            FROM oidc_authorization_requests a CROSS JOIN oidc_authorization_requests b WHERE a.id=$1 AND b.id=$2")
            .bind(flow.id).bind(replacement.id).fetch_one(&database.pool).await?;
        ensure!(immutable,"restart changed an immutable authorization request or inherited the old authenticated context");
        for old in [&first,&switched] {
            let page=interaction(&app,&replacement,Some(old)).await?;
            expect(&page,StatusCode::OK,"restart displays a fresh-login requirement")?;
            ensure!(page.body["status"]=="LOGIN_REQUIRED","restart adopted an already-existing global authentication instead of requiring a new proof");
        }
        let fresh=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let new_code=approve(&app,&fixture.employee,&mut replacement,&fresh).await?;
        let pair=tokens(&exchange_code(&app,&fixture.employee,&new_code).await?)?;
        verify_id(&pair.id,&jwks(&app).await?,fixture.employee.id,&flow.nonce,fixture.memberless.id)?;
        expect(&interaction(&app,&flow,Some(&first)).await?,StatusCode::CONFLICT,"consumed old flow cannot be revived after explicit restart")?;
        let denied=browser(&app,Method::POST,&format!("/oidc/interaction/{}",second.id),Some(&flow_cookies(&second,Some(&first))),Some(second_csrf),json!({"action":"deny"})).await?;
        expect(&denied,StatusCode::OK,"independent browser flow can be explicitly denied")?;
        let denied=reqwest::Url::parse(text(&denied.body,"redirectTo")?)?;
        ensure!(denied.query_pairs().any(|(key,value)|key=="error" && value=="access_denied")
            && denied.query_pairs().any(|(key,value)|key=="state" && value==second.state)
            && !denied.query_pairs().any(|(key,_)|key=="code"),"consent denial returned a code or lost its original state");
        for extra in [vec![("prompt","login".into())],vec![("max_age","0".into())]] {
            let forced=start_flow_with(&app,&fixture.employee,Some(&first),None,&extra).await?;
            let page=interaction(&app,&forced,Some(&first)).await?;
            expect(&page,StatusCode::OK,"prompt login or max_age zero interaction")?;
            ensure!(page.body["status"]=="LOGIN_REQUIRED","prompt login or max_age zero reused an old account authentication");
        }
        let mut none=authorize_parameters(&fixture.employee,"owned-prompt-state","owned-prompt-nonce");none.push(("prompt","none".into()));
        let reply=exchange(&app,Method::GET,&format!("/oidc/authorize?{}",encoded(&none)),None,String::new(),&[]).await?;
        authorization_error(&reply,&fixture.employee,"owned-prompt-state","login_required")?;
        let expired=start_flow(&app,&fixture.employee,Some(&fresh),None).await?;
        sqlx::query("UPDATE oidc_authorization_requests SET created_at=clock_timestamp()-interval '1 hour',expires_at=clock_timestamp()-interval '1 second' WHERE id=$1")
            .bind(expired.id).execute(&database.pool).await?;
        expect(&interaction(&app,&expired,Some(&fresh)).await?,StatusCode::CONFLICT,"expired authorization interaction cannot bind or approve a session")?;
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE09_TEST_DATABASE_URL and disposable database creation"]
async fn phase09_online_session_account_issuer_and_dependency_authority_fail_closed() -> Result<()>
{
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let global=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let unrelated=sso_login(&app,&fixture.administrator,PASSWORD).await?;
        let (_,initial)=approved_code(&app,&fixture.employee,&global).await?;
        let old=tokens(&exchange_code(&app,&fixture.employee,&initial).await?)?;
        let (_,pending)=approved_code(&app,&fixture.employee,&global).await?;
        let (_,customer_code)=approved_code(&app,&fixture.customer,&global).await?;
        let customer=tokens(&exchange_code(&app,&fixture.customer,&customer_code).await?)?;
        expect(&browser(&app,Method::POST,"/api/sso/logout",Some(&global.cookie),Some(&global.csrf),Value::Null).await?,
            StatusCode::OK,"real global SSO logout retires provider grants")?;
        for (client,pair) in [(&fixture.employee,&old),(&fixture.customer,&customer)] {
            expect(&userinfo(&app,&pair.access).await?,StatusCode::UNAUTHORIZED,"global logout immediately rejects provider access")?;
            oauth_error(&refresh(&app,client,&pair.refresh).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        }
        oauth_error(&exchange_code(&app,&fixture.employee,&pending).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        expect(&sso_session(&app,&unrelated).await?,StatusCode::OK,"another account survives global provider logout")?;
        let fresh=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let sibling=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let (_,code)=approved_code(&app,&fixture.employee,&fresh).await?;
        let fresh_pair=tokens(&exchange_code(&app,&fixture.employee,&code).await?)?;
        let (_,pending)=approved_code(&app,&fixture.employee,&fresh).await?;
        let (_,sibling_code)=approved_code(&app,&fixture.customer,&sibling).await?;
        let sibling_pair=tokens(&exchange_code(&app,&fixture.customer,&sibling_code).await?)?;
        let sid=session_id(&database.pool,&fresh).await?;
        sqlx::query("UPDATE sso_sessions SET authenticated_at=clock_timestamp()-interval '1 hour',expires_at=clock_timestamp()-interval '1 second' WHERE id=$1")
            .bind(sid).execute(&database.pool).await?;
        expect(&userinfo(&app,&fresh_pair.access).await?,StatusCode::UNAUTHORIZED,"original global sid expiry rejects a physically live access grant")?;
        oauth_error(&refresh(&app,&fixture.employee,&fresh_pair.refresh).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        oauth_error(&exchange_code(&app,&fixture.employee,&pending).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        expect(&userinfo(&app,&sibling_pair.access).await?,StatusCode::OK,"expiry of one global sid preserves another valid sid of the same account")?;
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(fixture.memberless.id).execute(&database.pool).await?;
        sqlx::query("UPDATE sso_sessions SET revoked_at=NULL WHERE id=$1").bind(session_id(&database.pool,&sibling).await?).execute(&database.pool).await?;
        sqlx::query("UPDATE oidc_grants SET revoked_at=NULL WHERE user_id=$1").bind(fixture.memberless.id).execute(&database.pool).await?;
        expect(&userinfo(&app,&sibling_pair.access).await?,StatusCode::UNAUTHORIZED,"authoritative account generation rejects restored physical grant and sid rows")?;
        oauth_error(&refresh(&app,&fixture.customer,&sibling_pair.refresh).await?,StatusCode::BAD_REQUEST,"invalid_grant")?;
        expect(&sso_session(&app,&unrelated).await?,StatusCode::OK,"account generation change preserves an unrelated account")?;
        let active=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let (_,active_code)=approved_code(&app,&fixture.employee,&active).await?;
        let active_pair=tokens(&exchange_code(&app,&fixture.employee,&active_code).await?)?;
        let (_,live_code)=approved_code(&app,&fixture.employee,&active).await?;
        let mut wrong_config=(*fixture.state.config).clone();
        wrong_config.oidc_issuer=Some("https://other-auth.example.invalid".into());
        wrong_config.sso_origin=Some("https://other-auth.example.invalid".into());
        let wrong_state=AppState::new(database.pool.clone(),Arc::new(wrong_config));
        ensure!(crate::oidc::initialize(&wrong_state).await.is_err(),"same database accepted a different issuer at startup initialization");
        let wrong_app=router(wrong_state);
        for path in ["/.well-known/openid-configuration","/oidc/jwks"] {
            expect(&call(&wrong_app,Method::GET,path,None,Value::Null).await?,StatusCode::SERVICE_UNAVAILABLE,"persisted issuer pin rejects a jointly changed SSO origin and provider issuer")?;
        }
        expect(&exchange_code(&wrong_app,&fixture.employee,&live_code).await?,StatusCode::SERVICE_UNAVAILABLE,"issuer mismatch cannot issue a new token")?;
        fixture.state.pool.close().await;
        for reply in [
            userinfo(&app,&active_pair.access).await?,
            exchange_code(&app,&fixture.employee,&live_code).await?,
            refresh(&app,&fixture.employee,&active_pair.refresh).await?,
            call(&app,Method::GET,"/.well-known/openid-configuration",None,Value::Null).await?,
            call(&app,Method::GET,"/oidc/jwks",None,Value::Null).await?,
        ] {
            expect(&reply,StatusCode::SERVICE_UNAVAILABLE,"unavailable provider authority cannot be interpreted as an invalid password or usable token")?;
            ensure!(reply.body.get("access_token").is_none() && reply.body.get("id_token").is_none()
                && reply.body.get("sub").is_none(),"failed provider authority returned partial authentication");
        }
        Ok(())
    }.await;
    database.finish(assertions).await
}

async fn wait_for_lock(pool: &PgPool, blocker: i32, pattern: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let observed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) AND query ILIKE $2)",
            )
            .bind(blocker)
            .bind(pattern)
            .fetch_one(pool)
            .await?;
            if observed {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| anyhow!("OIDC request did not reach its observed lock barrier"))?
}

async fn provider_snapshot(pool: &PgPool, user: Uuid) -> Result<Value> {
    // Never include this in an assertion message. It contains persisted hashes
    // and immutable nonce/state proofs compared exclusively in memory.
    Ok(sqlx::query_scalar("SELECT jsonb_build_object( \
        'user',(SELECT to_jsonb(u) FROM users u WHERE id=$1), \
        'sessions',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY id),'[]'::jsonb) FROM sso_sessions s WHERE user_id=$1), \
        'requests',(SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY id),'[]'::jsonb) FROM oidc_authorization_requests r WHERE user_id=$1), \
        'grants',(SELECT COALESCE(jsonb_agg(to_jsonb(g) ORDER BY id),'[]'::jsonb) FROM oidc_grants g WHERE user_id=$1), \
        'codes',(SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.id),'[]'::jsonb) FROM oidc_authorization_codes c JOIN oidc_grants g ON g.id=c.grant_id WHERE g.user_id=$1), \
        'access',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.id),'[]'::jsonb) FROM oidc_access_tokens t JOIN oidc_grants g ON g.id=t.grant_id WHERE g.user_id=$1), \
        'refresh',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.id),'[]'::jsonb) FROM oidc_refresh_tokens t JOIN oidc_grants g ON g.id=t.grant_id WHERE g.user_id=$1), \
        'keys',(SELECT COALESCE(jsonb_agg(to_jsonb(k) ORDER BY kid),'[]'::jsonb) FROM oidc_signing_keys k), \
        'audit',(SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY id),'[]'::jsonb) FROM audit_logs a WHERE user_id=$1))")
        .bind(user).fetch_one(pool).await?)
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE09_TEST_DATABASE_URL and disposable database creation"]
async fn phase09_code_exchange_rechecks_authority_after_real_lock_waits_and_rolls_back_audit_failure(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        for mutation in ["sid","account","client"] {
            let global=sso_login(&app,&fixture.memberless,PASSWORD).await?;
            let (_,code)=approved_code(&app,&fixture.employee,&global).await?;
            let sid=session_id(&database.pool,&global).await?;
            let mut blocker=database.pool.begin().await?;
            let pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
            sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(fixture.memberless.id).execute(&mut *blocker).await?;
            let pending_app=app.clone();let client=fixture.employee.clone();let pending_code=code.clone();
            let mut pending=tokio::spawn(async move {exchange_code(&pending_app,&client,&pending_code).await});
            if let Err(error)=wait_for_lock(&database.pool,pid,"%FROM users%FOR UPDATE%").await {
                blocker.rollback().await?;pending.abort();let _=pending.await;return Err(error);
            }
            match mutation {
                "sid"=>{sqlx::query("UPDATE sso_sessions SET revoked_at=clock_timestamp() WHERE id=$1").bind(sid).execute(&mut *blocker).await?;},
                "account"=>{crate::session::revoke_all_in_tx(&mut blocker,fixture.memberless.id,None).await?;},
                "client"=>{sqlx::query("UPDATE oidc_clients SET access_policy_version=access_policy_version+1,updated_at=clock_timestamp() WHERE id=$1")
                    .bind(fixture.employee.id).execute(&mut *blocker).await?;},
                _=>return Err(anyhow!("unknown owned OIDC lock-wait mutation")),
            }
            blocker.commit().await?;
            let reply=match tokio::time::timeout(Duration::from_secs(10),&mut pending).await {
                Ok(reply)=>reply??,
                Err(_)=>{pending.abort();let _=pending.await;return Err(anyhow!("OIDC exchange did not finish after lock release"));}
            };
            oauth_error(&reply,StatusCode::BAD_REQUEST,"invalid_grant")?;
            let clean:(bool,i64,i64,i64)=sqlx::query_as("SELECT \
                (SELECT consumed_at IS NULL FROM oidc_authorization_codes WHERE token_hash=$1), \
                (SELECT count(*) FROM oidc_access_tokens), (SELECT count(*) FROM oidc_refresh_tokens), \
                (SELECT count(*) FROM audit_logs WHERE user_id=$2 AND action='OIDC_EXCHANGE_CODE')")
                .bind(hash::sha256_hex(&code)).bind(fixture.memberless.id).fetch_one(&database.pool).await?;
            ensure!(clean==(true,0,0,0),"rejected stale exchange consumed a code or partially persisted credentials/audit");
        }
        let global=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let (_,code)=approved_code(&app,&fixture.employee,&global).await?;
        let before=provider_snapshot(&database.pool,fixture.memberless.id).await?;
        sqlx::raw_sql("CREATE FUNCTION phase09_reject_exchange_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
            BEGIN IF NEW.action='OIDC_EXCHANGE_CODE' THEN RAISE EXCEPTION 'owned OIDC audit failure'; END IF; RETURN NEW; END $$; \
            CREATE TRIGGER phase09_reject_exchange_audit BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION phase09_reject_exchange_audit();")
            .execute(&database.pool).await?;
        let failed=exchange_code(&app,&fixture.employee,&code).await?;
        ensure!(failed.status.is_server_error() && failed.body.get("access_token").is_none() && failed.body.get("id_token").is_none(),
            "audit failure returned a successful or partial OIDC credential");
        ensure!(provider_snapshot(&database.pool,fixture.memberless.id).await?==before,
            "OIDC audit failure partially committed code consumption, signed-key expiry, tokens or account state");
        sqlx::query("DROP TRIGGER phase09_reject_exchange_audit ON audit_logs").execute(&database.pool).await?;
        let pair=tokens(&exchange_code(&app,&fixture.employee,&code).await?)?;
        expect(&userinfo(&app,&pair.access).await?,StatusCode::OK,"rolled-back code remains redeemable after the real audit dependency recovers")?;
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE09_TEST_DATABASE_URL and disposable database creation"]
async fn phase09_signing_key_overlap_retirement_and_emergency_revocation_use_real_verification_windows(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::with_token_ttl(&database.pool,5).await?;
        let app=fixture.app();
        let global=sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let (first_flow,first_code)=approved_code(&app,&fixture.employee,&global).await?;
        let first=tokens(&exchange_code(&app,&fixture.employee,&first_code).await?)?;
        ensure!(jsonwebtoken::decode_header(&first.id)?.kid.as_deref()==Some("phase09-a"),"initial ID Token did not use the active provisioned key");
        let first_claims=verify_id(&first.id,&jwks(&app).await?,fixture.employee.id,&first_flow.nonce,fixture.memberless.id)?;
        expect(&call(&app,Method::DELETE,"/api/oidc/keys/phase09-a",Some(&fixture.admin),Value::Null).await?,
            StatusCode::CONFLICT,"active signing key cannot be normally retired")?;
        expect(&call(&app,Method::POST,"/api/oidc/keys/activate",Some(&fixture.admin),json!({"kid":"phase09-b"})).await?,
            StatusCode::OK,"published next key can be activated")?;
        let overlapping=jwks(&app).await?;
        for kid in ["phase09-a","phase09-b"] {
            ensure!(overlapping["keys"].as_array().is_some_and(|keys|keys.iter().any(|key|key["kid"]==kid)),"rotation removed a required overlapping public key");
        }
        let (second_flow,second_code)=approved_code(&app,&fixture.employee,&global).await?;
        let second=tokens(&exchange_code(&app,&fixture.employee,&second_code).await?)?;
        ensure!(jsonwebtoken::decode_header(&second.id)?.kid.as_deref()==Some("phase09-b"),"newly issued ID Token did not move to the active next key");
        verify_id(&first.id,&overlapping,fixture.employee.id,&first_flow.nonce,fixture.memberless.id)?;
        verify_id(&second.id,&overlapping,fixture.employee.id,&second_flow.nonce,fixture.memberless.id)?;
        let retained:bool=sqlx::query_scalar("SELECT NOT active AND last_signed_exp>=to_timestamp($1) AND retired_at IS NULL AND revoked_at IS NULL FROM oidc_signing_keys WHERE kid='phase09-a'")
            .bind(first_claims["exp"].as_i64().ok_or_else(||anyhow!("signed ID Token omitted its expiry"))? as f64).fetch_one(&database.pool).await?;
        ensure!(retained,"old key publication did not retain its actual greatest signed-token window");
        expect(&call(&app,Method::DELETE,"/api/oidc/keys/phase09-a",Some(&fixture.admin),Value::Null).await?,
            StatusCode::CONFLICT,"inactive old public key must remain during its token verification window")?;
        tokio::time::timeout(Duration::from_secs(75),async {
            loop {
                let safe:bool=sqlx::query_scalar("SELECT clock_timestamp()>last_signed_exp+interval '60 seconds' FROM oidc_signing_keys WHERE kid='phase09-a'")
                    .fetch_one(&database.pool).await?;
                if safe {return Ok::<_,anyhow::Error>(());}
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }).await.map_err(|_|anyhow!("actual old-key verification window did not elapse"))??;
        expect(&call(&app,Method::DELETE,"/api/oidc/keys/phase09-a",Some(&fixture.admin),Value::Null).await?,
            StatusCode::OK,"old public key retires after its real token window and safety margin")?;
        let retired=jwks(&app).await?;
        ensure!(retired["keys"].as_array().is_some_and(|keys|keys.iter().all(|key|key["kid"]!="phase09-a")&&keys.iter().any(|key|key["kid"]=="phase09-b")),
            "normal retirement kept the old key available or removed the active next key");
        ensure!(verify_id(&first.id,&retired,fixture.employee.id,&first_flow.nonce,fixture.memberless.id).is_err(),"retired-key ID Token still validates against fresh JWKS");
        expect(&call(&app,Method::POST,"/api/oidc/keys/activate",Some(&fixture.admin),json!({"kid":"phase09-a"})).await?,
            StatusCode::CONFLICT,"retired kid cannot be reactivated")?;
        let (new_flow,new_code)=approved_code(&app,&fixture.employee,&global).await?;
        let new=tokens(&exchange_code(&app,&fixture.employee,&new_code).await?)?;
        verify_id(&new.id,&retired,fixture.employee.id,&new_flow.nonce,fixture.memberless.id)?;
        expect(&call(&app,Method::POST,"/api/oidc/keys/phase09-b/revoke",Some(&fixture.admin),Value::Null).await?,
            StatusCode::OK,"explicit emergency operation can revoke the active key")?;
        let emergency=jwks(&app).await?;
        ensure!(verify_id(&new.id,&emergency,fixture.employee.id,&new_flow.nonce,fixture.memberless.id).is_err(),"emergency-revoked kid remains trusted in newly fetched JWKS");
        let (_,pending)=approved_code(&app,&fixture.employee,&global).await?;
        let failed=exchange_code(&app,&fixture.employee,&pending).await?;
        oauth_error(&failed,StatusCode::SERVICE_UNAVAILABLE,"temporarily_unavailable")?;
        let unconsumed:bool=sqlx::query_scalar("SELECT consumed_at IS NULL FROM oidc_authorization_codes WHERE token_hash=$1")
            .bind(hash::sha256_hex(&pending)).fetch_one(&database.pool).await?;
        ensure!(unconsumed,"missing active signing key partially consumed the valid authorization code");
        for key in overlapping["keys"].as_array().ok_or_else(||anyhow!("JWKS omitted its key array"))? {
            ensure!(["d","p","q","dp","dq","qi","privateKeyPem"].iter().all(|name|key.get(*name).is_none()),"JWKS exposed private RSA components");
        }
        Ok(())
    }.await;
    database.finish(assertions).await
}

async fn wait_for_clock(pool: &PgPool, deadline: chrono::DateTime<chrono::Utc>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let expired: bool = sqlx::query_scalar("SELECT clock_timestamp()>$1::timestamptz")
                .bind(deadline)
                .fetch_one(pool)
                .await?;
            if expired {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| anyhow!("database clock did not cross the original OIDC authority deadline"))?
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE09_TEST_DATABASE_URL and disposable database creation"]
async fn phase09_code_and_authentication_freshness_expiring_during_audit_wait_roll_back(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        sqlx::raw_sql("CREATE FUNCTION phase09_hold_authorize_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
            BEGIN IF NEW.action='OIDC_AUTHORIZE' THEN PERFORM pg_advisory_xact_lock(8039,9); END IF; RETURN NEW; END $$; \
            CREATE TRIGGER phase09_hold_authorize_audit BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION phase09_hold_authorize_audit();")
            .execute(&database.pool).await?;
        for mode in ["manual-code","automatic-code","manual-age","automatic-age"] {
            let age=mode.ends_with("age");let automatic=mode.starts_with("automatic");
            let mut config=(*fixture.state.config).clone();
            config.oidc_code_ttl_seconds=if age {120}else{1};
            let app=router(AppState::new(database.pool.clone(),Arc::new(config)));
            let global=sso_login(&app,&fixture.memberless,PASSWORD).await?;
            let sid=session_id(&database.pool,&global).await?;
            let mut flow=if automatic {
                // An existing grant permits this next online request to use the
                // automatic code branch. Offline access itself still requires
                // the explicit approval used for this seed.
                let (seed,_)=approved_code(&app,&fixture.employee,&global).await?;
                Some(seed)
            } else {
                let extras=if age {vec![("max_age","1".into())]}else{Vec::new()};
                let mut flow=start_flow_with(&app,&fixture.employee,Some(&global),None,&extras).await?;
                let page=interaction(&app,&flow,Some(&global)).await?;
                expect(&page,StatusCode::OK,"fresh original authentication is eligible before the audit barrier")?;
                ensure!(page.body["status"]=="CONSENT_REQUIRED","fresh max_age proof expired before approval could exercise the commit guard");
                flow.csrf=text(&page.body,"csrfToken")?.into();
                Some(flow)
            };
            let mut blocker=database.pool.begin().await?;
            let pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
            sqlx::query("SELECT pg_advisory_xact_lock(8039,9)").execute(&mut *blocker).await?;
            let before=provider_snapshot(&database.pool,fixture.memberless.id).await?;
            let pending_app=app.clone();
            let client=fixture.employee.clone();
            let current=flow.take().ok_or_else(||anyhow!("owned deadline test omitted its interaction"))?;
            let mut pending=tokio::spawn(async move {
                if automatic {
                    let mut fields=authorize_parameters(&client,"owned-auto-deadline-state","owned-auto-deadline-nonce");
                    fields.iter_mut().find(|(key,_)|*key=="scope").ok_or_else(||anyhow!("owned authorization parameters omitted scope"))?.1="openid profile".into();
                    fields.push(("prompt","none".into()));if age {fields.push(("max_age","1".into()));}
                    exchange(&pending_app,Method::GET,&format!("/oidc/authorize?{}",encoded(&fields)),None,String::new(),
                        &[("cookie",&flow_cookies(&current,Some(&global)))]).await
                } else {
                    browser(&pending_app,Method::POST,&format!("/oidc/interaction/{}",current.id),Some(&flow_cookies(&current,Some(&global))),Some(&current.csrf),json!({"action":"approve"})).await
                }
            });
            if let Err(error)=wait_for_lock(&database.pool,pid,"%INSERT INTO audit_logs%").await {
                blocker.rollback().await?;pending.abort();let _=pending.await;return Err(error);
            }
            let observed:chrono::DateTime<chrono::Utc>=sqlx::query_scalar("SELECT clock_timestamp()").fetch_one(&database.pool).await?;
            let deadline=if age {
                let deadline:chrono::DateTime<chrono::Utc>=sqlx::query_scalar("SELECT authenticated_at+interval '1 second' FROM sso_sessions WHERE id=$1")
                    .bind(sid).fetch_one(&database.pool).await?;
                if observed>=deadline {
                    blocker.rollback().await?;pending.abort();let _=pending.await;
                    return Err(anyhow!("original max_age expired before the audit barrier; final freshness guard was not exercised"));
                }
                deadline
            } else {
                // The code was created before this observed audit INSERT. Its
                // configured one-second lifetime is therefore certainly over
                // once the database clock passes observation + 1.05 seconds.
                observed+chrono::Duration::milliseconds(1050)
            };
            if let Err(error)=wait_for_clock(&database.pool,deadline).await {
                blocker.rollback().await?;pending.abort();let _=pending.await;return Err(error);
            }
            blocker.commit().await?;
            let reply=match tokio::time::timeout(Duration::from_secs(10),&mut pending).await {
                Ok(reply)=>reply??,
                Err(_)=>{pending.abort();let _=pending.await;return Err(anyhow!("OIDC authorization did not finish after its observed audit barrier"));}
            };
            if automatic {oauth_error(&reply,StatusCode::BAD_REQUEST,"invalid_grant")?;}
            else {expect(&reply,StatusCode::UNAUTHORIZED,"manual approval rejects expiry before commit")?;}
            ensure!(reply.body.get("redirectTo").is_none()&&!reply.headers.contains_key(header::LOCATION),"expired authorization returned an already-dead code callback");
            ensure!(provider_snapshot(&database.pool,fixture.memberless.id).await?==before,
                "code or original auth freshness expiry partially committed grant/code, consumed flow, key state or successful audit");
        }
        Ok(())
    }.await;
    database.finish(assertions).await
}
