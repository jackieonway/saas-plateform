//! Phase 10 BFF contracts use real provider handlers and owned disposable databases.
//! Only IDENTITY_PHASE10_TEST_DATABASE_URL is read; no dotenv or existing database is migrated.
use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
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
mod centralized;

const SECRET: &str = "identity-phase10-owned-fixture-signing-key";
const PASSWORD: &str = "Owned-Phase10-Fixture-Password9!";
const SERVICE_SECRET: &str = "Owned-Phase10-Application-Service-Fixture9!";
const REDIRECT: &str = "https://console.example.invalid/auth/callback";
const CUSTOMER_REDIRECT: &str = "https://customer.example.invalid/auth/callback";
const BROWSER_COOKIE: &str = "__Host-saas_oidc_browser";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let raw = std::env::var("IDENTITY_PHASE10_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE10_TEST_DATABASE_URL is required"))?;
        let url = reqwest::Url::parse(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 10 database configuration"))?;
        let port = url
            .port()
            .ok_or_else(|| anyhow!("Phase 10 administration requires an explicit port"))?;
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
                "Phase 10 administration requires an explicit loopback host, port and account without redirection");
        let options = PgConnectOptions::from_str(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 10 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres")
                && loopback(options.get_host())
                && options.get_port() == port
                && !options.get_username().is_empty(),
            "Phase 10 administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated Phase 10 administration connection failed"))?;
        let name = format!("identity_phase10_{}", uuid7::new_uuid().simple());
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
                return Err(anyhow!("owned Phase 10 database connection failed"));
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
        let suffix = self.name.strip_prefix("identity_phase10_");
        ensure!(
            suffix.is_some_and(|value| value.len() == 32
                && value.bytes().all(|ch| ch.is_ascii_hexdigit())
                && Uuid::parse_str(value).is_ok_and(uuid7::is_uuid_v7)),
            "refusing cleanup outside an owned Phase 10 database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!(
            "identity_phase10_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
            self.name, assertions.is_ok(), removed.is_ok()
        );
        assertions?;
        removed?;
        ensure!(closed, "owned Phase 10 pool did not close");
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
    .map_err(|_| anyhow!("owned Phase 10 HTTP request timed out"))??;
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
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin,real_name) VALUES($1,$2,$3,$4,'Owned Phase 10 account')")
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
                "identity_phase10_keys_{}.json",
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
            {"kid":"phase10-a","privateKeyPem":KEY_A_PEM,"n":KEY_A_N,"e":"AQAB"},
            {"kid":"phase10-b","privateKeyPem":KEY_B_PEM,"n":KEY_B_N,"e":"AQAB"}
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
                .and_then(|name| name.strip_prefix("identity_phase10_keys_"))
                .and_then(|name| name.strip_suffix(".json"))
                .is_some_and(|suffix| {
                    suffix.len() == 32
                        && suffix.bytes().all(|ch| ch.is_ascii_hexdigit())
                        && Uuid::parse_str(suffix).is_ok_and(uuid7::is_uuid_v7)
                });
        if owned {
            let removed = std::fs::remove_file(&self.path).is_ok();
            eprintln!("identity_phase10_keyset_cleanup removed={removed}");
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

#[derive(Clone, Copy)]
struct Member {
    id: Uuid,
    org: Uuid,
    kind: &'static str,
}

struct Fixture {
    state: AppState,
    admin: String,
    admin_refresh: String,
    memberless: Account,
    customer_only: Account,
    dual: Account,
    members: [Member; 3],
    business: Uuid,
    service_client: String,
    employee: Client,
    customer: Client,
    _keys: KeyFile,
}

impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        Self::with_token_ttl(pool, 300).await
    }

    async fn with_token_ttl(pool: &PgPool, token_ttl: i64) -> Result<Self> {
        // Apply the accepted Phase 6 target shape only to this owned, newly
        // migrated fixture. Production rollout receipt gates stay unchanged.
        sqlx::query(
            "ALTER TABLE memberships DROP CONSTRAINT IF EXISTS memberships_user_id_organization_id_key",
        )
        .execute(pool)
        .await?;
        let keys = KeyFile::new()?;
        let tenant = uuid7::new_uuid();
        let org = uuid7::new_uuid();
        let application = uuid7::new_uuid();
        let menu = uuid7::new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned Phase 10 tenant','admin-tenant',true)")
            .bind(tenant).execute(pool).await?;
        sqlx::query(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned Phase 10 root')",
        )
        .bind(org)
        .bind(tenant)
        .execute(pool)
        .await?;
        let administrator = account(pool, "phase10-administrator", true).await?;
        let memberless = account(pool, "phase10-memberless", false).await?;
        let customer_only = account(pool, "phase10-customer-only", false).await?;
        let dual = account(pool, "phase10-dual", false).await?;
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,'CUSTOMER','ENABLED',1)")
            .bind(uuid7::new_uuid()).bind(customer_only.id).bind(org).execute(pool).await?;
        let second_org = uuid7::new_uuid();
        sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,'Owned Phase 10 second organization')")
            .bind(second_org).bind(tenant).bind(org).execute(pool).await?;
        let members = [
            Member {
                id: uuid7::new_uuid(),
                org,
                kind: "EMPLOYEE",
            },
            Member {
                id: uuid7::new_uuid(),
                org,
                kind: "CUSTOMER",
            },
            Member {
                id: uuid7::new_uuid(),
                org: second_org,
                kind: "EMPLOYEE",
            },
        ];
        for member in members {
            sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,$4,'ENABLED',1)")
                .bind(member.id).bind(dual.id).bind(member.org).bind(member.kind).execute(pool).await?;
        }
        let member = uuid7::new_uuid();
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,'EMPLOYEE','ENABLED',1)")
            .bind(member).bind(administrator.id).bind(org).execute(pool).await?;
        sqlx::query("INSERT INTO applications(id,name,code,allowed_member_types) VALUES($1,'Owned Phase 10 management','phase10-management',ARRAY['EMPLOYEE'])")
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
        sqlx::query(
            "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
        )
        .bind(second_org)
        .bind(application)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,path) VALUES($1,$2,'MENU','Owned Phase 10 management','system:oauth','/system/oauth/tokens')")
            .bind(menu).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(menu)
            .execute(pool)
            .await?;
        for permission in ["system:user", "system:application"] {
            let granted_menu = uuid7::new_uuid();
            sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,path) VALUES($1,$2,'MENU','Owned Phase 10 management permission',$3,$4)")
                .bind(granted_menu).bind(application).bind(permission).bind(format!("/owned/{permission}")).execute(pool).await?;
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(tenant)
                .bind(granted_menu)
                .execute(pool)
                .await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org)
                .bind(granted_menu)
                .execute(pool)
                .await?;
        }
        let business = uuid7::new_uuid();
        sqlx::query("INSERT INTO applications(id,name,code,allowed_member_types) VALUES($1,'Owned Phase 10 business','phase10-business',ARRAY['EMPLOYEE','CUSTOMER'])")
            .bind(business).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(business)
            .execute(pool)
            .await?;
        for organization in [org, second_org] {
            sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(organization).bind(business).execute(pool).await?;
        }
        let service_id = uuid7::new_uuid();
        let service_client = format!("phase10-service-{}", service_id.simple());
        sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_type,client_secret_hash,subject_mode,allowed_member_types,application_id,grant_types,scopes) \
            VALUES($1,$2,'Owned Phase 10 application service','CONFIDENTIAL',$3,'MACHINE','{}',$4,'client_credentials','profile')")
            .bind(service_id).bind(&service_client).bind(hash::sha256_hex(SERVICE_SECRET)).bind(business).execute(pool).await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
            .bind(org)
            .bind(menu)
            .execute(pool)
            .await?;
        let settings = Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/identity-phase10".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
            // This owned APP_ENV=test database deliberately exercises the real
            // lifecycle API. Production readiness/receipt gates stay unchanged.
            ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), "true".into()),
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
        sqlx::query("INSERT INTO oidc_provider_settings(singleton,issuer,created_at) VALUES(true,$1,clock_timestamp())")
            .bind(ORIGIN).execute(pool).await?;
        // Provision the isolated authority fixture before any runtime login.
        // Administrator setup must use the same SSO/BFF proof as every console user.
        for (kid, n, active) in [("phase10-a", KEY_A_N, true), ("phase10-b", KEY_B_N, false)] {
            sqlx::query("INSERT INTO oidc_signing_keys(kid,n,e,published_at,active,activated_at) VALUES($1,$2,'AQAB',clock_timestamp(),$3,CASE WHEN $3 THEN clock_timestamp() ELSE NULL END)")
                .bind(kid).bind(n).bind(active).execute(pool).await?;
        }
        let employee = provision_client(pool, "PLATFORM_CONSOLE", &["EMPLOYEE"], REDIRECT).await?;
        let customer =
            provision_client(pool, "CUSTOMER_PORTAL", &["CUSTOMER"], CUSTOMER_REDIRECT).await?;
        let management_client =
            provision_client(pool, "PLATFORM_CONSOLE", &["EMPLOYEE"], REDIRECT).await?;
        let mut fixture = Self {
            state: AppState::new(pool.clone(), Arc::new(settings)),
            admin: String::new(),
            admin_refresh: "psr_retired_public_fixture".into(),
            memberless,
            customer_only,
            dual,
            members,
            business,
            service_client,
            employee: management_client,
            customer,
            _keys: keys,
        };
        let bff = Bff::new(&fixture)?;
        let (_, signed_in) = bff.login(&fixture, &administrator).await?;
        let selected = selected(
            &bff,
            &signed_in,
            Member {
                id: member,
                org,
                kind: "EMPLOYEE",
            },
        )
        .await?;
        fixture.admin = text(&selected.body, "accessToken")?.into();
        fixture.state = bff.state.clone();
        fixture.employee = employee;
        Ok(fixture)
    }

    fn app(&self) -> Router {
        router(self.state.clone())
    }
}

async fn provision_client(
    pool: &PgPool,
    purpose: &str,
    types: &[&str],
    redirect: &str,
) -> Result<Client> {
    let id = uuid7::new_uuid();
    let secret = crate::sso::opaque_secret()?;
    let name = format!("Owned Phase 10 {purpose}");
    let types = types
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    sqlx::query("INSERT INTO oidc_clients(id,name,purpose,secret_hash,allowed_member_types,redirect_uris,post_logout_redirect_uris,refresh_enabled,status,access_policy_version,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,true,'ENABLED',1,clock_timestamp(),clock_timestamp())")
        .bind(id).bind(&name).bind(purpose).bind(crate::util::hash::sha256_hex(&secret)).bind(types)
        .bind(vec![redirect.to_owned()]).bind(vec!["https://console.example.invalid/signed-out".to_owned()])
        .execute(pool).await?;
    Ok(Client {
        id,
        secret,
        redirect: redirect.into(),
        name,
    })
}

async fn new_client(
    app: &Router,
    admin: &str,
    purpose: &str,
    types: &[&str],
    redirect: &str,
) -> Result<Client> {
    let name = format!("Owned Phase 10 {purpose}");
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

const CONSOLE_ORIGIN: &str = "https://console.example.invalid";
const LOGIN_COOKIE: &str = "__Host-saas_console_login";
const CONSOLE_COOKIE: &str = "__Host-saas_console";

async fn console(
    app: &Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    body: Value,
) -> Result<Reply> {
    let mut headers = vec![
        ("origin", CONSOLE_ORIGIN),
        ("host", "console.example.invalid"),
        ("content-type", "application/json"),
    ];
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

struct ConsoleLogin {
    cookie: String,
    state: String,
    nonce: String,
    provider: reqwest::Url,
    destination: String,
}

async fn console_login(
    app: &Router,
    client: &Client,
    destination: &str,
    existing: Option<&str>,
) -> Result<ConsoleLogin> {
    let path = format!("/auth/login?{}", encoded(&[("return", destination.into())]));
    let started = console(app, Method::GET, &path, existing, None, Value::Null).await?;
    expect(
        &started,
        StatusCode::SEE_OTHER,
        "real BFF login starts the configured provider flow",
    )?;
    let target = started
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("BFF login omitted its provider redirect"))?;
    let provider = reqwest::Url::parse(target)?;
    ensure!(
        provider.origin().ascii_serialization() == ORIGIN
            && provider.path() == "/oidc/authorize"
            && provider.fragment().is_none(),
        "BFF login redirected to an untrusted identity endpoint"
    );
    let parameter = |name: &str| -> Result<String> {
        let matching = provider
            .query_pairs()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        ensure!(
            matching.len() == 1,
            "BFF authorization omitted or repeated a required public parameter"
        );
        Ok(matching[0].clone())
    };
    ensure!(
        parameter("client_id")? == client.id.to_string()
            && parameter("redirect_uri")? == client.redirect
            && parameter("response_type")? == "code"
            && parameter("code_challenge_method")? == "S256"
            && !provider
                .query_pairs()
                .any(|(key, _)| key == "code_verifier" || key == "client_secret"),
        "BFF login broadened the registered RP or exposed a server-only credential"
    );
    let challenge = parameter("code_challenge")?;
    ensure!(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(challenge)
            .is_ok_and(|bytes| bytes.len() == 32),
        "BFF login did not generate a canonical S256 challenge"
    );
    let cookie = if let Some(existing) = existing {
        if started.headers.contains_key(header::SET_COOKIE) {
            ensure!(
                response_cookie(&started, LOGIN_COOKIE)? == existing,
                "opening another tab rotated the shared console login binding"
            );
        }
        existing.into()
    } else {
        response_cookie(&started, LOGIN_COOKIE)?
    };
    cookie_secret(&cookie)?;
    Ok(ConsoleLogin {
        cookie,
        state: parameter("state")?,
        nonce: parameter("nonce")?,
        provider,
        destination: destination.into(),
    })
}

async fn provider_approval(
    provider: &Router,
    client: &Client,
    login: &ConsoleLogin,
    sso: &Sso,
    existing: Option<&str>,
) -> Result<(Flow, String)> {
    let mut cookies = sso.cookie.clone();
    if let Some(existing) = existing {
        cookies = format!("{cookies}; {existing}");
    }
    let target = format!(
        "{}?{}",
        login.provider.path(),
        login
            .provider
            .query()
            .ok_or_else(|| anyhow!("BFF redirect omitted standard OIDC parameters"))?
    );
    let started = exchange(
        provider,
        Method::GET,
        &target,
        None,
        String::new(),
        &[("origin", ORIGIN), ("cookie", &cookies)],
    )
    .await?;
    expect(
        &started,
        StatusCode::SEE_OTHER,
        "actual provider accepts the BFF-generated PKCE request",
    )?;
    let location = started
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("real provider omitted its interaction redirect"))?;
    let location = reqwest::Url::parse(ORIGIN)?.join(location)?;
    ensure!(
        location.path() == "/sso/login",
        "BFF login unexpectedly bypassed its explicit offline consent interaction"
    );
    let id = location
        .query_pairs()
        .find(|(key, _)| key == "flow")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| anyhow!("real provider omitted its interaction identity"))?;
    let browser = if let Some(existing) = existing {
        existing.into()
    } else {
        response_cookie(&started, BROWSER_COOKIE)?
    };
    let mut flow = Flow {
        id: Uuid::parse_str(&id)?,
        cookie: browser,
        csrf: String::new(),
        state: login.state.clone(),
    };
    let code = approve(provider, client, &mut flow, sso).await?;
    Ok((flow, code))
}

fn callback_path(login: &ConsoleLogin, code: &str) -> String {
    format!(
        "/auth/callback?{}",
        encoded(&[
            ("code", code.into()),
            ("state", login.state.clone()),
            ("iss", ORIGIN.into())
        ])
    )
}

struct ConsoleSession {
    cookie: String,
}

async fn callback(app: &Router, login: &ConsoleLogin, code: &str) -> Result<Reply> {
    // A real cross-site top-level OIDC redirect is a navigation GET, which
    // ordinarily carries no Origin. State and its host-scoped login binding
    // must authenticate this callback independently of write-route CSRF rules.
    let reply = exchange(
        app,
        Method::GET,
        &callback_path(login, code),
        None,
        String::new(),
        &[
            ("host", "console.example.invalid"),
            ("cookie", &login.cookie),
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-dest", "document"),
        ],
    )
    .await?;
    no_store(&reply)?;
    Ok(reply)
}

async fn completed_login(app: &Router, login: &ConsoleLogin, code: &str) -> Result<ConsoleSession> {
    let completed = callback(app, login, code).await?;
    expect(
        &completed,
        StatusCode::SEE_OTHER,
        "BFF validates the real provider code and ID Token",
    )?;
    let redirect = completed
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("BFF callback omitted its fixed same-origin destination"))?;
    ensure!(
        redirect == login.destination,
        "BFF callback replaced the initiating tab's permitted destination"
    );
    let cookie = response_cookie(&completed, CONSOLE_COOKIE)?;
    cookie_secret(&cookie)?;
    ensure!(
        cookie_secret(&cookie)? != cookie_secret(&login.cookie)?,
        "BFF session adopted the preauthentication browser binding as an authenticated secret"
    );
    Ok(ConsoleSession { cookie })
}

async fn console_session(app: &Router, session: &ConsoleSession) -> Result<Reply> {
    console(
        app,
        Method::GET,
        "/auth/session",
        Some(&session.cookie),
        None,
        Value::Null,
    )
    .await
}

// The adapter changes only transport. Successful exchanges still execute the
// provider's real discovery, PKCE, signing, refresh and account-proof handlers.
#[derive(Clone, Copy)]
#[repr(u8)]
enum Fault {
    Healthy = 0,
    Nonce,
    Issuer,
    Audience,
    Subject,
    Signature,
    Algorithm,
    UnknownKid,
    MissingKid,
    UserInfoSubject,
    InactiveProof,
    Pkce,
    Unavailable,
    RefreshResponseLost,
    SessionId,
    DiscoveryEndpoint,
    EarlyRealId,
    LogoutResponseLost,
}

#[derive(Clone, Default)]
struct CapturedTokens {
    access: String,
    refresh: String,
    id: String,
}

#[derive(Clone, PartialEq, Eq)]
struct LogoutAttempt {
    endpoint: usize,
    request_id: Option<Uuid>,
    token_digest: String,
}

struct ProviderTransport {
    selection_extension: Mutex<Option<Value>>,
    app: Router,
    short_app: Router,
    fault: AtomicU8,
    counts: [AtomicU64; 8],
    delays: [AtomicU64; 8],
    refreshes: AtomicU64,
    captured: Mutex<CapturedTokens>,
    pause_proof: AtomicBool,
    proof_entered: tokio::sync::Notify,
    proof_release: tokio::sync::Notify,
    shorter_access_exp: AtomicI64,
    pause_userinfo: AtomicBool,
    userinfo_entered: tokio::sync::Notify,
    userinfo_release: tokio::sync::Notify,
    logout_attempts: Mutex<Vec<LogoutAttempt>>,
    pause_logout: AtomicBool,
    logout_entered: tokio::sync::Notify,
    logout_release: tokio::sync::Notify,
}

fn endpoint_index(endpoint: crate::bff::Endpoint) -> usize {
    use crate::bff::Endpoint;
    match endpoint {
        Endpoint::Discovery => 0,
        Endpoint::Jwks => 1,
        Endpoint::Token => 2,
        Endpoint::UserInfo => 3,
        Endpoint::Introspect => 4,
        Endpoint::Revoke => 5,
        Endpoint::GlobalLogout => 6,
        Endpoint::ContextInteractions => 7,
    }
}

impl ProviderTransport {
    fn new(app: Router, short_app: Router) -> Self {
        Self {
            selection_extension: Mutex::new(None),
            app,
            short_app,
            fault: AtomicU8::new(Fault::Healthy as u8),
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
            delays: std::array::from_fn(|_| AtomicU64::new(0)),
            refreshes: AtomicU64::new(0),
            captured: Mutex::new(CapturedTokens::default()),
            pause_proof: AtomicBool::new(false),
            proof_entered: tokio::sync::Notify::new(),
            proof_release: tokio::sync::Notify::new(),
            shorter_access_exp: AtomicI64::new(0),
            pause_userinfo: AtomicBool::new(false),
            userinfo_entered: tokio::sync::Notify::new(),
            userinfo_release: tokio::sync::Notify::new(),
            logout_attempts: Mutex::new(Vec::new()),
            pause_logout: AtomicBool::new(false),
            logout_entered: tokio::sync::Notify::new(),
            logout_release: tokio::sync::Notify::new(),
        }
    }

    fn fault(&self, value: Fault) {
        self.fault.store(value as u8, Ordering::SeqCst);
    }

    fn count(&self, endpoint: crate::bff::Endpoint) -> u64 {
        self.counts[endpoint_index(endpoint)].load(Ordering::SeqCst)
    }

    fn captured(&self) -> Result<CapturedTokens> {
        self.captured
            .lock()
            .map(|value| value.clone())
            .map_err(|_| anyhow!("owned transport capture lock failed"))
    }

    fn logout_attempts(&self) -> Result<Vec<LogoutAttempt>> {
        self.logout_attempts
            .lock()
            .map(|value| value.clone())
            .map_err(|_| anyhow!("owned logout capture lock failed"))
    }

    fn mutate_id(&self, body: &mut Value, fault: u8) -> Result<()> {
        let token = text(body, "id_token")?;
        let mut pieces = token.split('.');
        let _ = pieces.next();
        let payload = pieces
            .next()
            .ok_or_else(|| anyhow!("real provider omitted signed ID Token claims"))?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload)?;
        let mut claims: Value = serde_json::from_slice(&bytes)?;
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some("phase10-a".into());
        if fault == Fault::Nonce as u8 {
            claims["nonce"] = json!("owned-negative-nonce");
        } else if fault == Fault::Issuer as u8 {
            claims["iss"] = json!("https://other-auth.example.invalid");
        } else if fault == Fault::Audience as u8 {
            claims["aud"] = json!(uuid7::new_uuid());
        } else if fault == Fault::Subject as u8 {
            claims["sub"] = json!(Uuid::nil());
        } else if fault == Fault::SessionId as u8 {
            claims["sid"] = json!(uuid7::new_uuid());
        } else if fault == Fault::UnknownKid as u8 {
            header.kid = Some("phase10-unpublished".into());
        } else if fault == Fault::MissingKid as u8 {
            header.kid = None;
        }
        let encoding = if fault == Fault::Algorithm as u8 {
            header.alg = jsonwebtoken::Algorithm::HS256;
            jsonwebtoken::EncodingKey::from_secret(b"owned-negative-algorithm-fixture")
        } else {
            jsonwebtoken::EncodingKey::from_rsa_pem(if fault == Fault::Signature as u8 {
                KEY_B_PEM.as_bytes()
            } else {
                KEY_A_PEM.as_bytes()
            })?
        };
        body["id_token"] = json!(jsonwebtoken::encode(&header, &claims, &encoding)?);
        Ok(())
    }
}

impl crate::bff::OidcTransport for ProviderTransport {
    fn send<'a>(&'a self, mut outgoing: crate::bff::OidcRequest) -> crate::bff::OidcFuture<'a> {
        Box::pin(async move {
            use crate::bff::{Endpoint, OidcResponse};
            use crate::error::AppError;
            let index = endpoint_index(outgoing.endpoint);
            self.counts[index].fetch_add(1, Ordering::SeqCst);
            let refreshing = index == endpoint_index(Endpoint::Token)
                && outgoing
                    .form
                    .iter()
                    .any(|(key, value)| key == "grant_type" && value == "refresh_token");
            if refreshing {
                self.refreshes.fetch_add(1, Ordering::SeqCst);
            }
            let fault = self.fault.load(Ordering::SeqCst);
            if fault == Fault::Unavailable as u8 {
                return Err(AppError::Unavailable);
            }
            let delay = self.delays[index].load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            if fault == Fault::Pkce as u8 && index == endpoint_index(Endpoint::Token) {
                for (key, value) in &mut outgoing.form {
                    if key == "code_verifier" {
                        *value = "owned-negative-verifier-0123456789012345678901234567890123456789"
                            .into();
                    }
                }
            }
            let parameters = outgoing
                .form
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone()))
                .collect::<Vec<_>>();
            let mut request = Request::builder()
                .method(if outgoing.endpoint.is_post() {
                    Method::POST
                } else {
                    Method::GET
                })
                .uri(outgoing.endpoint.path())
                .header(
                    "content-type",
                    if outgoing.json.is_some() {
                        "application/json"
                    } else {
                        "application/x-www-form-urlencoded"
                    },
                )
                .extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 240], 30038))));
            if let Some(authorization) = &outgoing.authorization {
                request = request.header(header::AUTHORIZATION, authorization);
            }
            let request = request
                .body(Body::from(if let Some(body) = outgoing.json {
                    serde_json::to_vec(&body).map_err(|_| AppError::Unavailable)?
                } else {
                    encoded(&parameters).into_bytes()
                }))
                .map_err(|_| AppError::Unavailable)?;
            let provider =
                if index == endpoint_index(Endpoint::Token) && fault == Fault::EarlyRealId as u8 {
                    &self.short_app
                } else {
                    &self.app
                };
            let reply = provider
                .clone()
                .oneshot(request)
                .await
                .map_err(|_| AppError::Unavailable)?;
            let status = reply.status().as_u16();
            let bytes = to_bytes(reply.into_body(), 65536)
                .await
                .map_err(|_| AppError::Unavailable)?;
            if matches!(outgoing.endpoint, Endpoint::Revoke | Endpoint::GlobalLogout)
                && status == 200
            {
                let token = outgoing
                    .form
                    .iter()
                    .find(|(key, _)| key == "token")
                    .map(|(_, value)| value.as_str())
                    .ok_or(AppError::Unavailable)?;
                let request_id = outgoing
                    .form
                    .iter()
                    .find(|(key, _)| key == "request_id")
                    .map(|(_, value)| Uuid::parse_str(value))
                    .transpose()
                    .map_err(|_| AppError::Unavailable)?;
                self.logout_attempts
                    .lock()
                    .map_err(|_| AppError::Unavailable)?
                    .push(LogoutAttempt {
                        endpoint: index,
                        request_id,
                        token_digest: hash::sha256_hex(token),
                    });
                if self.pause_logout.swap(false, Ordering::SeqCst) {
                    // The actual provider transaction has already committed;
                    // only delivery of its real empty-200 reply is paused.
                    self.logout_entered.notify_one();
                    self.logout_release.notified().await;
                }
                if fault == Fault::LogoutResponseLost as u8 {
                    return Err(AppError::Unavailable);
                }
            }
            if bytes.is_empty() {
                // RFC 7009 and account logout legitimately return an empty
                // 200 response. Transport must preserve that real wire body.
                return Ok(OidcResponse {
                    status,
                    body: Vec::new(),
                });
            }
            let mut body: Value =
                serde_json::from_slice(&bytes).map_err(|_| AppError::Unavailable)?;
            if index == endpoint_index(Endpoint::Introspect) {
                let earlier = self.shorter_access_exp.load(Ordering::SeqCst);
                if earlier > 0 && body["active"] == true {
                    // A negative transport case can only shorten the genuine
                    // live proof, never invent or extend a valid authority.
                    if let Some(expiry) = body["exp"].as_i64() {
                        body["exp"] = json!(expiry.min(earlier));
                    }
                }
            }
            if index == endpoint_index(Endpoint::Introspect)
                && self.pause_proof.swap(false, Ordering::SeqCst)
            {
                self.proof_entered.notify_one();
                self.proof_release.notified().await;
            }
            if index == endpoint_index(Endpoint::Token) && status == 200 {
                if !refreshing {
                    if let Some(extension) = self
                        .selection_extension
                        .lock()
                        .map_err(|_| AppError::Unavailable)?
                        .take()
                    {
                        body["context_selection"] = extension;
                    }
                }
                if fault == Fault::EarlyRealId as u8 {
                    // Isolate ID expiry from access expiry using only genuine
                    // provider-signed credentials for the same real grant.
                    // The short-TTL provider produced this ID; a real refresh
                    // through the normal provider produces longer-lived access.
                    let original_id = body["id_token"].clone();
                    let refresh = body["refresh_token"]
                        .as_str()
                        .ok_or(AppError::Unavailable)?;
                    let followup = Request::builder()
                        .method(Method::POST)
                        .uri(Endpoint::Token.path())
                        .header("content-type", "application/x-www-form-urlencoded")
                        .header(
                            header::AUTHORIZATION,
                            outgoing
                                .authorization
                                .as_deref()
                                .ok_or(AppError::Unavailable)?,
                        )
                        .extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 241], 30038))))
                        .body(Body::from(encoded(&[
                            ("grant_type", "refresh_token".into()),
                            ("refresh_token", refresh.into()),
                        ])))
                        .map_err(|_| AppError::Unavailable)?;
                    let followup = self
                        .app
                        .clone()
                        .oneshot(followup)
                        .await
                        .map_err(|_| AppError::Unavailable)?;
                    if followup.status() != StatusCode::OK {
                        return Err(AppError::Unavailable);
                    }
                    let bytes = to_bytes(followup.into_body(), 65536)
                        .await
                        .map_err(|_| AppError::Unavailable)?;
                    body = serde_json::from_slice(&bytes).map_err(|_| AppError::Unavailable)?;
                    body["id_token"] = original_id;
                }
                *self.captured.lock().map_err(|_| AppError::Unavailable)? = CapturedTokens {
                    access: body["access_token"].as_str().unwrap_or("").into(),
                    refresh: body["refresh_token"].as_str().unwrap_or("").into(),
                    id: body["id_token"].as_str().unwrap_or("").into(),
                };
                if refreshing && fault == Fault::RefreshResponseLost as u8 {
                    // The provider has rotated. The caller cannot distinguish a
                    // lost reply from a crash after sending the original token.
                    return Err(AppError::Unavailable);
                }
                if matches!(fault, 1..=8 | 14) {
                    self.mutate_id(&mut body, fault)
                        .map_err(|_| AppError::Unavailable)?;
                }
            }
            if index == endpoint_index(Endpoint::UserInfo) && fault == Fault::UserInfoSubject as u8
            {
                body["sub"] = json!(uuid7::new_uuid());
            }
            if index == endpoint_index(Endpoint::UserInfo)
                && self.pause_userinfo.swap(false, Ordering::SeqCst)
            {
                self.userinfo_entered.notify_one();
                self.userinfo_release.notified().await;
            }
            if index == endpoint_index(Endpoint::Introspect) && fault == Fault::InactiveProof as u8
            {
                body = json!({"active":false});
            }
            if index == endpoint_index(Endpoint::Discovery)
                && fault == Fault::DiscoveryEndpoint as u8
            {
                body["token_endpoint"] = json!("https://untrusted.example.invalid/oidc/token");
            }
            Ok(OidcResponse {
                status,
                body: serde_json::to_vec(&body).map_err(|_| AppError::Unavailable)?,
            })
        })
    }
}

struct TokenKeyFile {
    path: PathBuf,
}
impl TokenKeyFile {
    fn new() -> Result<Self> {
        let artifact = Self {
            path: std::env::temp_dir().join(format!(
                "identity_phase10_cipher_{}.json",
                uuid7::new_uuid().simple()
            )),
        };
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        // Dedicated public test material; never a deployment or user key.
        let material = json!({"activeKid":"phase10-encryption","keys":[{"kid":"phase10-encryption","key":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([31_u8;32])}]});
        options
            .open(&artifact.path)?
            .write_all(material.to_string().as_bytes())?;
        Ok(artifact)
    }
}
impl Drop for TokenKeyFile {
    fn drop(&mut self) {
        let owned = self.path.parent() == Some(std::env::temp_dir().as_path())
            && self
                .path
                .file_name()
                .and_then(|value| value.to_str())
                .and_then(|value| value.strip_prefix("identity_phase10_cipher_"))
                .and_then(|value| value.strip_suffix(".json"))
                .is_some_and(|suffix| {
                    suffix.len() == 32
                        && suffix.bytes().all(|value| value.is_ascii_hexdigit())
                        && Uuid::parse_str(suffix).is_ok_and(uuid7::is_uuid_v7)
                });
        if owned {
            let removed = std::fs::remove_file(&self.path).is_ok();
            eprintln!("identity_phase10_cipher_cleanup removed={removed}");
        }
    }
}

struct Bff {
    state: AppState,
    app: Router,
    transport: Arc<ProviderTransport>,
    _key: TokenKeyFile,
}
impl Bff {
    fn new(fixture: &Fixture) -> Result<Self> {
        let key = TokenKeyFile::new()?;
        let mut config = (*fixture.state.config).clone();
        config.bff_oidc_issuer = Some(ORIGIN.into());
        config.bff_oidc_client_id = Some(fixture.employee.id);
        config.bff_oidc_client_secret = Some(fixture.employee.secret.clone());
        config.bff_oidc_redirect_uri = Some(REDIRECT.into());
        config.bff_token_keys = Some(Arc::new(crate::bff::CipherKeys::load(
            key.path
                .to_str()
                .ok_or_else(|| anyhow!("owned keyfile path is not UTF-8"))?,
        )?));
        let mut state = AppState::new(fixture.state.pool.clone(), Arc::new(config));
        let mut short_config = (*fixture.state.config).clone();
        short_config.oidc_token_ttl_seconds = 2;
        let short_provider = router(AppState::new(
            fixture.state.pool.clone(),
            Arc::new(short_config),
        ));
        let transport = Arc::new(ProviderTransport::new(fixture.app(), short_provider));
        state.bff_transport = transport.clone();
        let app = router(state.clone());
        Ok(Self {
            state,
            app,
            transport,
            _key: key,
        })
    }

    fn other_instance(&self) -> Router {
        let mut state = AppState::new(self.state.pool.clone(), self.state.config.clone());
        state.bff_transport = self.transport.clone();
        router(state)
    }

    async fn login(&self, fixture: &Fixture, account: &Account) -> Result<(Sso, ConsoleSession)> {
        let provider = fixture.app();
        let sso = sso_login(&provider, account, PASSWORD).await?;
        let login = console_login(&self.app, &fixture.employee, "/", None).await?;
        let (_, code) = provider_approval(&provider, &fixture.employee, &login, &sso, None).await?;
        Ok((sso, completed_login(&self.app, &login, &code).await?))
    }
}

fn no_upstream_credentials(reply: &Reply, captured: &CapturedTokens) -> Result<()> {
    let visible = serde_json::to_string(&reply.body)?;
    ensure!(
        !visible.contains("refreshToken")
            && !visible.contains("refresh_token")
            && !visible.contains("id_token")
            && [&captured.access, &captured.refresh, &captured.id]
                .into_iter()
                .filter(|value| !value.is_empty())
                .all(|value| !visible.contains(value)),
        "browser JSON exposed a server-only upstream credential"
    );
    Ok(())
}

fn rejected(reply: &Reply, step: &str) -> Result<()> {
    ensure!(
        reply.status.is_client_error(),
        "{step} unexpectedly accepted an invalid authentication proof"
    );
    ensure!(
        !reply
            .headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|value| value.starts_with(&format!("{CONSOLE_COOKIE}="))
                && !value.starts_with(&format!("{CONSOLE_COOKIE}=;"))),
        "invalid proof established an authenticated console cookie"
    );
    Ok(())
}

fn no_cookie_mutation(reply: &Reply) -> Result<()> {
    ensure!(
        !reply.headers.contains_key(header::SET_COOKIE),
        "logout or recovery response can mutate a newer login's fixed-name Cookie"
    );
    Ok(())
}

async fn cookie_reload(app: &Router, session: &ConsoleSession) -> Result<Reply> {
    let reply = exchange(
        app,
        Method::GET,
        "/auth/session",
        None,
        String::new(),
        &[
            ("host", "console.example.invalid"),
            ("sec-fetch-site", "same-origin"),
            ("cookie", &session.cookie),
        ],
    )
    .await?;
    no_store(&reply)?;
    Ok(reply)
}

fn version(reply: &Reply) -> Result<String> {
    let version = text(&reply.body, "version")?;
    ensure!(
        version.parse::<u64>().is_ok_and(|value| value > 0),
        "BFF version is not a positive decimal string"
    );
    Ok(version.into())
}

async fn choose(
    app: &Router,
    session: &ConsoleSession,
    snapshot: &Reply,
    member: Option<Member>,
) -> Result<Reply> {
    let mut body = json!({"expectedVersion":version(snapshot)?});
    if let Some(member) = member {
        body["membershipId"] = json!(member.id);
        body["organizationId"] = json!(member.org);
    }
    console(
        app,
        Method::POST,
        "/auth/context-token",
        Some(&session.cookie),
        Some(text(&snapshot.body, "csrfToken")?),
        body,
    )
    .await
}

async fn selected(bff: &Bff, session: &ConsoleSession, member: Member) -> Result<Reply> {
    let snapshot = console_session(&bff.app, session).await?;
    expect(
        &snapshot,
        StatusCode::OK,
        "authenticated BFF session before exact membership selection",
    )?;
    let context = choose(&bff.app, session, &snapshot, Some(member)).await?;
    expect(
        &context,
        StatusCode::OK,
        "explicit valid employee membership selection",
    )?;
    Ok(context)
}

async fn bff_id(pool: &PgPool, session: &ConsoleSession) -> Result<Uuid> {
    Ok(
        sqlx::query_scalar("SELECT id FROM bff_sessions WHERE token_hash=$1")
            .bind(hash::sha256_hex(cookie_secret(&session.cookie)?))
            .fetch_one(pool)
            .await?,
    )
}

async fn derived_app(app: &Router, context: &str) -> Result<String> {
    let reply = call(
        app,
        Method::POST,
        "/api/oauth/session-token",
        Some(context),
        json!({"appCode":"phase10-business"}),
    )
    .await?;
    expect(
        &reply,
        StatusCode::CREATED,
        "actual qiankun app token derivation from BFF context",
    )?;
    ensure!(
        reply.body.get("refreshToken").is_none(),
        "application derivation leaked platform refresh"
    );
    Ok(text(&reply.body, "accessToken")?.into())
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_real_provider_memberless_and_exact_employee_cookie_boundary() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;
        let admin_claims=crate::util::jwt::verify(&fixture.admin,SECRET).map_err(|_|anyhow!("owned administrator bearer is invalid"))?;
        let admin_sid=Uuid::parse_str(admin_claims.sid.as_deref().ok_or_else(||anyhow!("owned administrator has no platform sid"))?)?;
        let source:(String,String,i64)=sqlx::query_as("SELECT origin_issuer,origin_client_ref,origin_client_policy_version FROM platform_sessions WHERE id=$1")
            .bind(admin_sid).fetch_one(&db.pool).await?;
        let sources:(bool,bool,bool)=sqlx::query_as("SELECT platform_source_active($1,$2,1,'EMPLOYEE'),platform_source_active($3,$2,1,'EMPLOYEE'),platform_source_active($4,$5,$6,'EMPLOYEE')")
            .bind("https://wrong-issuer.example.invalid").bind(fixture.employee.id.to_string()).bind(ORIGIN)
            .bind(&source.0).bind(&source.1).bind(source.2).fetch_one(&db.pool).await?;
        ensure!(!sources.0&&sources.1&&sources.2&&source.0==ORIGIN,
            "platform source projection accepted an incorrect OIDC issuer or broke the exact SSO source");
        let retired = uuid7::new_uuid();
        sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) SELECT $1,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,'urn:saas:legacy-platform','platform-console-legacy',1 FROM platform_sessions WHERE id=$2")
            .bind(retired).bind(admin_sid).execute(&db.pool).await?;
        let retired_proofs: (bool,bool) = sqlx::query_as("SELECT platform_source_active('urn:saas:legacy-platform','platform-console-legacy',1,'EMPLOYEE'),platform_session_active($1)")
            .bind(retired).fetch_one(&db.pool).await?;
        ensure!(retired_proofs == (false,false), "historical password-mode sources must not authorize any consumer");
        let mut retired_claims = admin_claims.clone();
        retired_claims.sid = Some(retired.to_string());
        let retired_token = crate::util::jwt::sign(&retired_claims, SECRET)?;
        expect(&call(&fixture.app(),Method::GET,"/api/auth/profile",Some(&retired_token),Value::Null).await?,StatusCode::UNAUTHORIZED,"signed historical sessions cannot bypass the SSO boundary")?;
        let bff=Bff::new(&fixture)?;
        let (_,memberless)=bff.login(&fixture,&fixture.memberless).await?;
        let anonymous_context=console_session(&bff.app,&memberless).await?;
        expect(&anonymous_context,StatusCode::OK,"memberless account completes real OIDC authentication")?;
        ensure!(anonymous_context.body["account"]["id"]==json!(fixture.memberless.id)
            &&anonymous_context.body["memberships"]==json!([])&&anonymous_context.body["context"].is_null(),
            "memberless authentication synthesized a platform membership or context");
        let absent=choose(&bff.app,&memberless,&anonymous_context,None).await?;
        rejected(&absent,"memberless context minting")?;
        let synthesized:i64=sqlx::query_scalar("SELECT count(*) FROM memberships WHERE user_id=$1").bind(fixture.memberless.id).fetch_one(&db.pool).await?;
        ensure!(synthesized==0,"BFF created an implicit employee membership");
        let (_,customer_only)=bff.login(&fixture,&fixture.customer_only).await?;
        let customer_state=console_session(&bff.app,&customer_only).await?;
        expect(&customer_state,StatusCode::OK,"customer-only account authenticates without employee admission")?;
        ensure!(customer_state.body["memberships"]==json!([])&&customer_state.body["context"].is_null(),
            "customer-only account inherited an employee context");
        let (_,session)=bff.login(&fixture,&fixture.dual).await?;
        let initial=console_session(&bff.app,&session).await?;
        expect(&initial,StatusCode::OK,"dual account has authenticated BFF state")?;
        let members=initial.body["memberships"].as_array().ok_or_else(||anyhow!("BFF omitted membership choices"))?;
        ensure!(members.len()==2&&members.iter().all(|member|member["memberType"]=="EMPLOYEE")
            &&members.iter().any(|member|member["membershipId"]==json!(fixture.members[0].id))
            &&members.iter().any(|member|member["membershipId"]==json!(fixture.members[2].id))
            &&initial.body["context"].is_null(),"BFF merged same-organization customer privileges or chose an implicit organization");
        let customer=choose(&bff.app,&session,&initial,Some(fixture.members[1])).await?;
        rejected(&customer,"customer row in platform console")?;
        let mismatched=Member {org:fixture.members[2].org,..fixture.members[0]};
        rejected(&choose(&bff.app,&session,&initial,Some(mismatched)).await?,"membership with another organization")?;
        let context=choose(&bff.app,&session,&initial,Some(fixture.members[0])).await?;
        expect(&context,StatusCode::OK,"same-organization employee is selected by exact membership")?;
        let token=text(&context.body,"accessToken")?;
        let claims=crate::util::jwt::verify(token,SECRET).map_err(|_|anyhow!("BFF context bearer failed platform verification"))?;
        ensure!(claims.typ=="session"&&claims.sub==fixture.dual.id.to_string()&&claims.org==Some(fixture.members[0].org.to_string())
            &&claims.exp-claims.iat<=300&&claims.exp>claims.iat,"BFF minted a broadened or long-lived platform context");
        expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(token),Value::Null).await?,StatusCode::OK,"short context Bearer reaches CurrentUser")?;
        let captured=bff.transport.captured()?;
        no_upstream_credentials(&initial,&captured)?;
        no_upstream_credentials(&context,&captured)?;
        ensure!(!captured.access.is_empty()&&!captured.refresh.is_empty()&&!captured.id.is_empty(),"successful BFF bypassed actual provider token issuance");
        for forbidden in [&captured.id,&captured.access] {
            expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(forbidden),Value::Null).await?,StatusCode::UNAUTHORIZED,"OIDC credentials cannot authorize platform business APIs")?;
        }
        expect(&exchange(&bff.app,Method::GET,"/api/auth/profile",None,String::new(),&[("cookie",&session.cookie)]).await?,StatusCode::UNAUTHORIZED,"console Cookie alone cannot authorize business API")?;
        let stored:(String,String,Option<String>)=sqlx::query_as("SELECT access_ciphertext,refresh_ciphertext,platform_refresh_ciphertext FROM bff_sessions WHERE id=$1")
            .bind(bff_id(&db.pool,&session).await?).fetch_one(&db.pool).await?;
        ensure!(stored.0.starts_with("v1.phase10-encryption.")&&stored.1.starts_with("v1.phase10-encryption.")
            &&stored.2.as_deref().is_some_and(|value|value.starts_with("v1.phase10-encryption."))
            &&stored.0!=captured.access&&stored.1!=captured.refresh,"server-only credentials were persisted as plaintext");
        ensure!(bff.transport.count(crate::bff::Endpoint::Token)>=2&&bff.transport.count(crate::bff::Endpoint::Jwks)>=2
            &&bff.transport.count(crate::bff::Endpoint::Introspect)>=2,"happy path bypassed real provider token, JWKS or account proof");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_callback_state_replay_expiry_browser_and_multitab_generation() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;let bff=Bff::new(&fixture)?;let provider=fixture.app();
        let sso=sso_login(&provider,&fixture.dual,PASSWORD).await?;
        let login=console_login(&bff.app,&fixture.employee,"/system/users",None).await?;
        let (_,code)=provider_approval(&provider,&fixture.employee,&login,&sso,None).await?;
        let wrong_state=format!("/auth/callback?{}",encoded(&[("state",uuid7::new_uuid().simple().to_string()),("code",code.clone()),("iss",ORIGIN.into())]));
        rejected(&console(&bff.app,Method::GET,&wrong_state,Some(&login.cookie),None,Value::Null).await?,"wrong state")?;
        let untouched:bool=sqlx::query_scalar("SELECT consumed_at IS NULL FROM console_login_flows WHERE state_hash=$1").bind(hash::sha256_hex(&login.state)).fetch_one(&db.pool).await?;
        ensure!(untouched,"wrong state consumed another valid login flow");
        let other=console_login(&bff.app,&fixture.employee,"/",None).await?;
        rejected(&console(&bff.app,Method::GET,&callback_path(&login,&code),Some(&other.cookie),None,Value::Null).await?,"callback from another browser")?;
        for suffix in ["&state=owned-duplicate","&code=owned-duplicate","&iss=https%3A%2F%2Fother.example.invalid"] {
            rejected(&console(&bff.app,Method::GET,&format!("{}{suffix}",callback_path(&login,&code)),Some(&login.cookie),None,Value::Null).await?,"duplicate callback parameter")?;
        }
        let session=completed_login(&bff.app,&login,&code).await?;
        rejected(&callback(&bff.app,&login,&code).await?,"consumed callback replay")?;
        expect(&console_session(&bff.app,&session).await?,StatusCode::OK,"bad callback attempts preserve legitimate original flow")?;
        let expired=console_login(&bff.app,&fixture.employee,"/",None).await?;
        let (_,expired_code)=provider_approval(&provider,&fixture.employee,&expired,&sso,None).await?;
        sqlx::query("UPDATE console_login_flows SET expires_at=clock_timestamp()-interval '1 second',created_at=clock_timestamp()-interval '10 minutes' WHERE state_hash=$1")
            .bind(hash::sha256_hex(&expired.state)).execute(&db.pool).await?;
        rejected(&callback(&bff.app,&expired,&expired_code).await?,"expired login transaction")?;
        let first=console_login(&bff.app,&fixture.employee,"/first",None).await?;
        let second=console_login(&bff.app,&fixture.employee,"/second",Some(&first.cookie)).await?;
        ensure!(first.state!=second.state&&first.nonce!=second.nonce,"separate tabs reused state or nonce");
        let (_,first_code)=provider_approval(&provider,&fixture.employee,&first,&sso,None).await?;
        let (_,second_code)=provider_approval(&provider,&fixture.employee,&second,&sso,None).await?;
        let winner=completed_login(&bff.app,&second,&second_code).await?;
        rejected(&callback(&bff.app,&first,&first_code).await?,"stale browser generation in the first tab")?;
        expect(&console_session(&bff.app,&winner).await?,StatusCode::OK,"late callback cannot replace the winning tab session")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_rejects_real_token_claim_corruption_pkce_and_untrusted_discovery() -> Result<()> {
    let db = Database::create().await?;
    let result = async {
        let fixture = Fixture::create(&db.pool).await?;
        let bff = Bff::new(&fixture)?;
        let provider = fixture.app();
        let sso = sso_login(&provider, &fixture.dual, PASSWORD).await?;
        for fault in [
            Fault::Nonce,
            Fault::Issuer,
            Fault::Audience,
            Fault::Subject,
            Fault::SessionId,
            Fault::Signature,
            Fault::Algorithm,
            Fault::UnknownKid,
            Fault::MissingKid,
            Fault::UserInfoSubject,
            Fault::InactiveProof,
            Fault::Pkce,
        ] {
            bff.transport.fault(Fault::Healthy);
            let login = console_login(&bff.app, &fixture.employee, "/", None).await?;
            let (_, code) =
                provider_approval(&provider, &fixture.employee, &login, &sso, None).await?;
            let before = bff.transport.count(crate::bff::Endpoint::Jwks);
            bff.transport.fault(fault);
            let reply = callback(&bff.app, &login, &code).await?;
            rejected(&reply, "real provider credential with an invalid RP proof")?;
            ensure!(
                bff.transport.count(crate::bff::Endpoint::Jwks) - before <= 1,
                "untrusted ID Token triggered an unbounded JWKS retrieval"
            );
            let sessions: i64 =
                sqlx::query_scalar("SELECT count(*) FROM bff_sessions WHERE user_id=$1")
                    .bind(fixture.dual.id)
                    .fetch_one(&db.pool)
                    .await?;
            ensure!(
                sessions == 0,
                "failed RP proof partially committed a BFF session"
            );
        }
        bff.transport.fault(Fault::Healthy);
        let login = console_login(&bff.app, &fixture.employee, "/", None).await?;
        let (_, code) = provider_approval(&provider, &fixture.employee, &login, &sso, None).await?;
        bff.transport.fault(Fault::DiscoveryEndpoint);
        let reply = callback(&bff.app, &login, &code).await?;
        ensure!(
            matches!(
                reply.status,
                StatusCode::UNAUTHORIZED | StatusCode::SERVICE_UNAVAILABLE
            ),
            "RP accepted a discovery endpoint outside its exact trusted issuer"
        );
        bff.transport.fault(Fault::Healthy);
        // A customer RP is a distinct audience; it is never borrowed as the
        // employee console's admission source, even for a dual account.
        ensure!(
            fixture.customer.id != fixture.employee.id,
            "test clients unexpectedly share an RP identity"
        );
        let good = console_login(&bff.app, &fixture.employee, "/", None).await?;
        let (_, code) = provider_approval(&provider, &fixture.employee, &good, &sso, None).await?;
        let session = completed_login(&bff.app, &good, &code).await?;
        expect(
            &console_session(&bff.app, &session).await?,
            StatusCode::OK,
            "valid independent provider flow after negative proof cases",
        )?;
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_csrf_origin_context_renewal_switch_and_actual_consumers() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;let bff=Bff::new(&fixture)?;
        let (_,session)=bff.login(&fixture,&fixture.dual).await?;
        let initial=console_session(&bff.app,&session).await?;
        let body=json!({"expectedVersion":version(&initial)?,"membershipId":fixture.members[0].id,"organizationId":fixture.members[0].org});
        for origin in [None,Some("https://attacker.example.invalid")] {
            let mut headers=vec![("cookie",session.cookie.as_str()),("content-type","application/json"),("x-csrf-token",text(&initial.body,"csrfToken")?)];
            if let Some(origin)=origin {headers.push(("origin",origin));}
            let reply=exchange(&bff.app,Method::POST,"/auth/context-token",None,body.to_string(),&headers).await?;
            expect(&reply,StatusCode::FORBIDDEN,"BFF write requires exact console Origin")?;no_store(&reply)?;
        }
        expect(&console(&bff.app,Method::POST,"/auth/context-token",Some(&session.cookie),None,body.clone()).await?,StatusCode::FORBIDDEN,"BFF write requires session CSRF")?;
        expect(&console(&bff.app,Method::POST,"/auth/context-token",Some(&session.cookie),Some("wrong"),body.clone()).await?,StatusCode::FORBIDDEN,"BFF rejects incorrect CSRF")?;
        let duplicate=format!("{}; {}",session.cookie,session.cookie);
        rejected(&console(&bff.app,Method::GET,"/auth/session",Some(&duplicate),None,Value::Null).await?,"duplicate console binding")?;
        let first=choose(&bff.app,&session,&initial,Some(fixture.members[0])).await?;
        expect(&first,StatusCode::OK,"initial explicit context selection")?;
        let old_token=text(&first.body,"accessToken")?.to_owned();
        let first_snapshot=console_session(&bff.app,&session).await?;
        ensure!(version(&first_snapshot)?.parse::<u64>()?==version(&initial)?.parse::<u64>()?+1,"selection failed to advance BFF generation");
        let renewed=choose(&bff.app,&session,&first_snapshot,None).await?;
        expect(&renewed,StatusCode::OK,"in-memory short context bearer renewal")?;
        let after_renewal=console_session(&bff.app,&session).await?;
        ensure!(version(&after_renewal)?==version(&first_snapshot)?,"pure renewal changed BFF context generation");
        let app_token=derived_app(&bff.app,text(&renewed.body,"accessToken")?).await?;
        let app_context=call(&bff.app,Method::GET,"/api/apps/phase10-business/context",Some(&app_token),Value::Null).await?;
        expect(&app_context,StatusCode::OK,"unchanged qiankun context contract from actual app token")?;
        ensure!(app_context.body["contractVersion"]=="1.0"&&app_context.body["user"]["id"]==json!(fixture.dual.id)
            &&app_context.body["organization"]["id"]==json!(fixture.members[0].org),"application context changed consumer identity or wire version");
        let socket=call(&bff.app,Method::GET,"/api/notifications/socket-context",Some(&old_token),Value::Null).await?;
        expect(&socket,StatusCode::OK,"short context bearer drives actual notification socket context")?;
        ensure!(socket.body.as_object().is_some_and(|value|value.len()==6)
            &&socket.body["userId"]==json!(fixture.dual.id)&&socket.body["organizationId"]==json!(fixture.members[0].org),"notification public context leaked private BFF authority or crossed organization");
        let switched=choose(&bff.app,&session,&after_renewal,Some(fixture.members[2])).await?;
        expect(&switched,StatusCode::OK,"explicit switch to second employee membership")?;
        let switched_snapshot=console_session(&bff.app,&session).await?;
        ensure!(version(&switched_snapshot)?.parse::<u64>()?==version(&after_renewal)?.parse::<u64>()?+1,"context switch failed to advance BFF generation");
        expect(&choose(&bff.app,&session,&after_renewal,Some(fixture.members[0])).await?,StatusCode::CONFLICT,"stale tab cannot overwrite selected context")?;
        for path in ["/api/auth/profile","/api/notifications/socket-context"] {
            expect(&call(&bff.app,Method::GET,path,Some(&old_token),Value::Null).await?,StatusCode::UNAUTHORIZED,"old context is revoked after organization switch")?;
        }
        expect(&call(&bff.app,Method::GET,"/api/apps/phase10-business/context",Some(&app_token),Value::Null).await?,StatusCode::UNAUTHORIZED,"derived app context retires with its original platform context")?;
        expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(text(&switched.body,"accessToken")?),Value::Null).await?,StatusCode::OK,"new context remains active")?;
        let captured=bff.transport.captured()?;
        no_upstream_credentials(&switched,&captured)?;
        let as_bearer=exchange(&bff.app,Method::GET,"/auth/session",Some(&old_token),String::new(),&[("origin",CONSOLE_ORIGIN),("cookie",&session.cookie)]).await?;
        expect(&as_bearer,StatusCode::UNAUTHORIZED,"BFF cookie endpoints reject business Bearer ambiguity")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_member_and_target_app_policy_are_local_but_rp_policy_retires_source() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;let bff=Bff::new(&fixture)?;let management=fixture.app();
        let broad=new_client(&management,&fixture.admin,"ACCOUNT",&["CUSTOMER","EMPLOYEE"],"https://account.example.invalid/auth/callback").await?;
        let (_,root_session)=bff.login(&fixture,&fixture.dual).await?;
        let choices=console_session(&bff.app,&root_session).await?;
        expect(&choices,StatusCode::OK,"current RP lists its own employee admission despite a broader active client")?;
        let members=choices.body["memberships"].as_array().ok_or_else(||anyhow!("BFF omitted membership choices"))?;
        ensure!(members.len()==2&&members.iter().all(|member|member["memberType"]=="EMPLOYEE")
            &&members.iter().all(|member|member["membershipId"]!=json!(fixture.members[1].id)),
            "another client's dual-type admission expanded the platform RP's member choices");
        expect(&choose(&bff.app,&root_session,&choices,Some(fixture.members[1])).await?,StatusCode::FORBIDDEN,
            "another active ACCOUNT client cannot lend customer admission to the platform RP")?;
        let root=selected(&bff,&root_session,fixture.members[0]).await?;
        let root_token=text(&root.body,"accessToken")?.to_owned();
        let root_app=derived_app(&bff.app,&root_token).await?;
        let (_,second_session)=bff.login(&fixture,&fixture.dual).await?;
        let second=selected(&bff,&second_session,fixture.members[2]).await?;
        let second_token=text(&second.body,"accessToken")?.to_owned();
        let second_app=derived_app(&bff.app,&second_token).await?;
        for member in [fixture.members[1],fixture.members[0]] {
            let disabled=call(&management,Method::PUT,&format!("/api/memberships/{}/status",member.id),Some(&fixture.admin),
                json!({"status":"DISABLED","expectedIdentityVersion":1,"reason":"Owned Phase 10 local membership regression"})).await?;
            expect(&disabled,StatusCode::OK,"real membership administration with explicitly enabled lifecycle fixture")?;
            expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(&second_token),Value::Null).await?,StatusCode::OK,"another valid organization stays active after local member change")?;
            let expected=if member.kind=="CUSTOMER" {StatusCode::OK}else{StatusCode::UNAUTHORIZED};
            expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(&root_token),Value::Null).await?,expected,"same-organization employee remains independent of customer membership")?;
        }
        expect(&call(&bff.app,Method::GET,"/api/notifications/socket-context",Some(&root_token),Value::Null).await?,StatusCode::UNAUTHORIZED,"locally disabled employee loses notification context")?;
        expect(&call(&bff.app,Method::GET,"/api/apps/phase10-business/context",Some(&root_app),Value::Null).await?,StatusCode::UNAUTHORIZED,"locally disabled employee loses derived application context")?;
        let root_account=console_session(&bff.app,&root_session).await?;
        expect(&root_account,StatusCode::OK,"local member revocation preserves authenticated account")?;
        ensure!(root_account.body["context"].is_null(),"BFF kept a disabled employee context");
        let policy=call(&management,Method::PUT,&format!("/api/applications/{}/access-policy",fixture.business),Some(&fixture.admin),
            json!({"allowedMemberTypes":["CUSTOMER"],"accessPolicyVersion":1,"reason":"Owned Phase 10 target application policy regression"})).await?;
        ensure!(policy.status.is_success(),"real target application policy update failed");
        expect(&call(&bff.app,Method::GET,"/api/apps/phase10-business/context",Some(&second_app),Value::Null).await?,StatusCode::UNAUTHORIZED,"old target-app token retires after policy restriction")?;
        let denied=call(&bff.app,Method::POST,"/api/oauth/session-token",Some(&second_token),json!({"appCode":"phase10-business"})).await?;
        expect(&denied,StatusCode::FORBIDDEN,"new target-app derivation checks its current member policy")?;
        expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(&second_token),Value::Null).await?,StatusCode::OK,"organization context survives an unrelated target-app policy change")?;
        let before_rp=console_session(&bff.app,&second_session).await?;
        expect(&before_rp,StatusCode::OK,"original RP account proof is live before its policy change")?;
        let rp=call(&management,Method::PATCH,&format!("/api/oidc/clients/{}",fixture.employee.id),Some(&fixture.admin),json!({
            "name":fixture.employee.name,"purpose":"PLATFORM_CONSOLE","allowedMemberTypes":["EMPLOYEE"],"redirectUris":[REDIRECT],
            "postLogoutRedirectUris":["https://console.example.invalid/signed-out"],"refreshEnabled":true,"status":"DISABLED","expectedAccessPolicyVersion":1
        })).await?;
        ensure!(rp.status.is_success(),"real RP policy update failed");
        expect(&console_session(&bff.app,&second_session).await?,StatusCode::UNAUTHORIZED,"source RP policy invalidates its BFF account proof")?;
        expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(&second_token),Value::Null).await?,StatusCode::UNAUTHORIZED,"source RP policy retires old platform context")?;
        let listing=call(&management,Method::GET,"/api/oidc/clients",Some(&fixture.admin),Value::Null).await?;
        expect(&listing,StatusCode::OK,"real management listing confirms the independent broad client remains active")?;
        let broad_record=listing.body["records"].as_array().and_then(|records|records.iter().find(|record|record["id"]==json!(broad.id)))
            .ok_or_else(||anyhow!("independent ACCOUNT client disappeared from its owner listing"))?;
        ensure!(broad_record["purpose"]=="ACCOUNT"&&broad_record["status"]=="ENABLED"
            &&broad_record["allowedMemberTypes"]==json!(["CUSTOMER","EMPLOYEE"])&&broad_record["accessPolicyVersion"]==1,
            "source RP restriction changed the other client's active dual-type policy");
        expect(&choose(&bff.app,&second_session,&before_rp,Some(fixture.members[2])).await?,StatusCode::UNAUTHORIZED,
            "a retired platform RP cannot borrow the still-active dual-type ACCOUNT client to mint a context")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_platform_logout_preserves_sso_and_global_logout_retires_all_sids() -> Result<()> {
    let db = Database::create().await?;
    let result = async {
        let fixture = Fixture::create(&db.pool).await?;
        let bff = Bff::new(&fixture)?;
        let (sso_one, session_one) = bff.login(&fixture, &fixture.dual).await?;
        let first = selected(&bff, &session_one, fixture.members[0]).await?;
        let first_token = text(&first.body, "accessToken")?.to_owned();
        let (sso_two, session_two) = bff.login(&fixture, &fixture.dual).await?;
        ensure!(
            session_id(&db.pool, &sso_one).await? != session_id(&db.pool, &sso_two).await?,
            "logout fixture failed to create independent global sessions"
        );
        let second = selected(&bff, &session_two, fixture.members[2]).await?;
        let second_token = text(&second.body, "accessToken")?.to_owned();
        let app_token = derived_app(&bff.app, &second_token).await?;
        for (session, scope) in [(&session_one, "PLATFORM"), (&session_two, "GLOBAL")] {
            let current = console_session(&bff.app, session).await?;
            let reply = console(
                &bff.app,
                Method::POST,
                "/auth/logout",
                Some(&session.cookie),
                Some(text(&current.body, "csrfToken")?),
                json!({"expectedVersion":version(&current)?,"scope":scope}),
            )
            .await?;
            expect(&reply, StatusCode::OK, "explicit BFF logout command")?;
            ensure!(
                reply.body["redirectTo"] == "/login?signedOut=1",
                "logout changed fixed signed-out navigation"
            );
            no_cookie_mutation(&reply)?;
            expect(
                &console_session(&bff.app, session).await?,
                StatusCode::UNAUTHORIZED,
                "logged-out console session remains retired",
            )?;
            if scope == "PLATFORM" {
                expect(
                    &sso_session(&fixture.app(), &sso_one).await?,
                    StatusCode::OK,
                    "platform logout preserves original global authentication",
                )?;
                expect(
                    &sso_session(&fixture.app(), &sso_two).await?,
                    StatusCode::OK,
                    "platform logout preserves another global session",
                )?;
                expect(
                    &call(
                        &bff.app,
                        Method::GET,
                        "/api/auth/profile",
                        Some(&first_token),
                        Value::Null,
                    )
                    .await?,
                    StatusCode::UNAUTHORIZED,
                    "platform logout retires its own context",
                )?;
                expect(
                    &call(
                        &bff.app,
                        Method::GET,
                        "/api/auth/profile",
                        Some(&second_token),
                        Value::Null,
                    )
                    .await?,
                    StatusCode::OK,
                    "platform logout preserves another BFF context",
                )?;
            }
        }
        for sso in [&sso_one, &sso_two] {
            expect(
                &sso_session(&fixture.app(), sso).await?,
                StatusCode::UNAUTHORIZED,
                "global logout reaches every global sid for the account",
            )?;
        }
        expect(
            &call(
                &bff.app,
                Method::GET,
                "/api/auth/profile",
                Some(&second_token),
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
            "global logout retires platform context",
        )?;
        expect(
            &call(
                &bff.app,
                Method::GET,
                "/api/apps/phase10-business/context",
                Some(&app_token),
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
            "global logout retires actual derived app context",
        )?;
        ensure!(
            bff.transport.count(crate::bff::Endpoint::Revoke) == 1
                && bff.transport.count(crate::bff::Endpoint::GlobalLogout) == 1,
            "logout commands crossed the provider's RP versus account-owner boundary"
        );
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_shared_authority_budget_failure_closed_and_machine_compatibility() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;let bff=Bff::new(&fixture)?;let provider=fixture.app();
        let sso=sso_login(&provider,&fixture.dual,PASSWORD).await?;
        let login=console_login(&bff.app,&fixture.employee,"/",None).await?;
        let (_,code)=provider_approval(&provider,&fixture.employee,&login,&sso,None).await?;
        bff.transport.delays[endpoint_index(crate::bff::Endpoint::Discovery)].store(2000,Ordering::SeqCst);
        bff.transport.delays[endpoint_index(crate::bff::Endpoint::Jwks)].store(2000,Ordering::SeqCst);
        let started=std::time::Instant::now();
        let timeout=callback(&bff.app,&login,&code).await?;
        let elapsed=started.elapsed();
        expect(&timeout,StatusCode::SERVICE_UNAVAILABLE,"entire callback chain shares one authority budget")?;
        ensure!(elapsed>=Duration::from_millis(2700)&&elapsed<Duration::from_millis(3900),
            "callback restarted its three-second budget for each upstream endpoint");
        let sessions:i64=sqlx::query_scalar("SELECT count(*) FROM bff_sessions WHERE user_id=$1").bind(fixture.dual.id).fetch_one(&db.pool).await?;
        ensure!(sessions==0,"authority timeout partially committed authenticated state");
        for delay in &bff.transport.delays {delay.store(0,Ordering::SeqCst);}
        let (_,session)=bff.login(&fixture,&fixture.dual).await?;
        let context=selected(&bff,&session,fixture.members[0]).await?;
        let token=text(&context.body,"accessToken")?;
        let application=derived_app(&bff.app,token).await?;
        bff.transport.fault(Fault::Unavailable);
        expect(&console_session(&bff.app,&session).await?,StatusCode::SERVICE_UNAVAILABLE,"BFF account reads fail closed during authority outage")?;
        for path in ["/api/auth/profile","/api/notifications/socket-context"] {
            expect(&call(&bff.app,Method::GET,path,Some(token),Value::Null).await?,StatusCode::SERVICE_UNAVAILABLE,"business consumers cannot trust cached BFF proof during outage")?;
        }
        expect(&call(&bff.app,Method::GET,"/api/apps/phase10-business/context",Some(&application),Value::Null).await?,StatusCode::SERVICE_UNAVAILABLE,"app consumer returns dependency outage distinctly")?;
        expect(&call(&bff.app,Method::POST,"/api/auth/login",None,json!({"username":fixture.dual.username,"password":PASSWORD})).await?,StatusCode::NOT_FOUND,"retired password login remains absent during outage")?;
        expect(&call(&bff.app,Method::POST,"/api/auth/context",Some(token),json!({"membershipId":fixture.members[0].id,"organizationId":fixture.members[0].org})).await?,StatusCode::NOT_FOUND,"retired credential selection remains absent")?;
        rejected(&form(&bff.app,"/oauth/token",&[("grant_type","refresh_token".into()),("refresh_token",fixture.admin_refresh.clone())]).await?,"public legacy platform refresh is explicitly disabled")?;
        let machine=form(&bff.app,"/oauth/token",&[("grant_type","client_credentials".into()),("client_id",fixture.service_client.clone()),("client_secret",SERVICE_SECRET.into())]).await?;
        expect(&machine,StatusCode::OK,"genuine machine grant retains its independent legacy contract")?;
        bff.transport.fault(Fault::Healthy);
        expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(token),Value::Null).await?,StatusCode::OK,"healthy authority restores valid original proof without password fallback")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_cross_instance_refresh_single_winner_and_lost_reply_never_replays() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;let bff=Bff::new(&fixture)?;
        let (_,session)=bff.login(&fixture,&fixture.dual).await?;
        let context=selected(&bff,&session,fixture.members[0]).await?;
        let original_context=text(&context.body,"accessToken")?.to_owned();
        let id=bff_id(&db.pool,&session).await?;
        let before:(String,String,chrono::DateTime<chrono::Utc>,i64)=sqlx::query_as("SELECT access_ciphertext,refresh_ciphertext,expires_at,version FROM bff_sessions WHERE id=$1")
            .bind(id).fetch_one(&db.pool).await?;
        sqlx::query("UPDATE bff_sessions SET access_expires_at=clock_timestamp()+interval '10 seconds' WHERE id=$1").bind(id).execute(&db.pool).await?;
        bff.transport.delays[endpoint_index(crate::bff::Endpoint::Token)].store(250,Ordering::SeqCst);
        let other=bff.other_instance();
        let (one,two)=tokio::join!(console_session(&bff.app,&session),console_session(&other,&session));
        let (one,two)=(one?,two?);
        ensure!([one.status,two.status].contains(&StatusCode::OK)
            &&[one.status,two.status].iter().all(|status|matches!(*status,StatusCode::OK|StatusCode::SERVICE_UNAVAILABLE)),
            "legitimate concurrent refresh produced an invalid authentication result");
        ensure!(bff.transport.refreshes.load(Ordering::SeqCst)==1,"two BFF instances sent the same rotating refresh to the provider");
        let after:(String,String,chrono::DateTime<chrono::Utc>,i64,bool,Option<Uuid>)=sqlx::query_as("SELECT access_ciphertext,refresh_ciphertext,expires_at,version,refresh_uncertain,refresh_lease_id FROM bff_sessions WHERE id=$1")
            .bind(id).fetch_one(&db.pool).await?;
        ensure!(after.0!=before.0&&after.1!=before.1&&after.2==before.2&&after.3==before.3&&!after.4&&after.5.is_none(),
            "refresh changed context generation/absolute expiry or failed to atomically rotate encrypted tokens");
        let grant_alive:bool=sqlx::query_scalar("SELECT g.revoked_at IS NULL FROM oidc_grants g JOIN bff_sessions b ON b.oidc_grant_id=g.id WHERE b.id=$1")
            .bind(id).fetch_one(&db.pool).await?;
        ensure!(grant_alive,"normal BFF concurrency triggered provider refresh reuse revocation");
        expect(&call(&other,Method::GET,"/api/auth/profile",Some(&original_context),Value::Null).await?,StatusCode::OK,"refresh retains existing exact platform context")?;
        bff.transport.delays[endpoint_index(crate::bff::Endpoint::Token)].store(0,Ordering::SeqCst);
        let (_,uncertain)=bff.login(&fixture,&fixture.dual).await?;
        let uncertain_id=bff_id(&db.pool,&uncertain).await?;
        let old=bff.transport.captured()?;
        sqlx::query("UPDATE bff_sessions SET access_expires_at=clock_timestamp()+interval '10 seconds' WHERE id=$1").bind(uncertain_id).execute(&db.pool).await?;
        let refreshes=bff.transport.refreshes.load(Ordering::SeqCst);
        bff.transport.fault(Fault::RefreshResponseLost);
        expect(&console_session(&bff.app,&uncertain).await?,StatusCode::SERVICE_UNAVAILABLE,"lost actual provider refresh reply is indeterminate")?;
        let consumed:bool=sqlx::query_scalar("SELECT used_at IS NOT NULL FROM oidc_refresh_tokens WHERE token_hash=$1")
            .bind(hash::sha256_hex(&old.refresh)).fetch_one(&db.pool).await?;
        ensure!(consumed,"lost-reply fixture did not actually rotate the provider token");
        bff.transport.fault(Fault::Healthy);
        expect(&console_session(&other,&uncertain).await?,StatusCode::SERVICE_UNAVAILABLE,"another BFF instance rejects an uncertain refresh outcome")?;
        // Reproduce durable state after a process dies between external send
        // and its local failure update. Expired lease is not a retry license.
        sqlx::query("UPDATE bff_sessions SET refresh_uncertain=false,refresh_started_at=clock_timestamp()-interval '1 hour' WHERE id=$1 AND refresh_lease_id IS NOT NULL")
            .bind(uncertain_id).execute(&db.pool).await?;
        expect(&console_session(&other,&uncertain).await?,StatusCode::SERVICE_UNAVAILABLE,"expired in-flight lease requires reauthentication")?;
        let failed_closed:bool=sqlx::query_scalar("SELECT refresh_uncertain FROM bff_sessions WHERE id=$1").bind(uncertain_id).fetch_one(&db.pool).await?;
        ensure!(failed_closed&&bff.transport.refreshes.load(Ordering::SeqCst)==refreshes+1,
            "expired/crashed refresh lease resent the original single-use token");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_inflight_old_current_user_app_and_notification_proof_cannot_survive_switch(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;let bff=Bff::new(&fixture)?;
        let (_,session)=bff.login(&fixture,&fixture.dual).await?;
        for path in ["/api/auth/profile","/api/apps/phase10-business/context","/api/notifications/socket-context"] {
            let selected=selected(&bff,&session,fixture.members[0]).await?;
            let context=text(&selected.body,"accessToken")?.to_owned();
            let bearer=if path.contains("/apps/"){derived_app(&bff.app,&context).await?}else{context};
            let snapshot=console_session(&bff.app,&session).await?;
            bff.transport.pause_proof.store(true,Ordering::SeqCst);
            let request_app=bff.app.clone();
            let pending=tokio::spawn(async move {call(&request_app,Method::GET,path,Some(&bearer),Value::Null).await});
            if tokio::time::timeout(Duration::from_secs(1),bff.transport.proof_entered.notified()).await.is_err() {
                pending.abort();return Err(anyhow!("consumer did not reach actual provider proof barrier"));
            }
            let changed=async {
                let mut tx=db.pool.begin().await?;
                sqlx::query("SET LOCAL lock_timeout='250ms'").execute(&mut *tx).await?;
                sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(fixture.dual.id).execute(&mut *tx).await?;
                tx.rollback().await?;
                let switched=choose(&bff.app,&session,&snapshot,Some(fixture.members[2])).await?;
                expect(&switched,StatusCode::OK,"another tab can switch while an old proof is in flight")?;
                Ok::<(),anyhow::Error>(())
            }.await;
            bff.transport.proof_release.notify_one();
            let stale=tokio::time::timeout(Duration::from_secs(3),pending).await.map_err(|_|anyhow!("paused consumer did not finish after proof release"))???;
            changed?;
            expect(&stale,StatusCode::UNAUTHORIZED,"proof started before context switch cannot revive the retired original platform session")?;
        }
        Ok(())
    }.await;
    db.finish(result).await
}

async fn waiting_for_user_lock(pool: &PgPool) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid() AND wait_event_type='Lock' AND query LIKE '%FROM users WHERE id=$1 FOR UPDATE%')")
            .fetch_one(pool).await?;
        if waiting {
            return Ok(());
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "context did not wait on the held account lock after actual remote proof"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_post_network_account_lock_rechecks_original_sid_and_account_generation() -> Result<()>
{
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;let bff=Bff::new(&fixture)?;
        let (sso,session)=bff.login(&fixture,&fixture.dual).await?;
        let snapshot=console_session(&bff.app,&session).await?;
        let original_sid=session_id(&db.pool,&sso).await?;
        let id=bff_id(&db.pool,&session).await?;
        let mut owner=db.pool.begin().await?;
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(fixture.dual.id).execute(&mut *owner).await?;
        let pending_app=bff.app.clone();let pending_cookie=session.cookie.clone();
        let body=json!({"expectedVersion":version(&snapshot)?,"membershipId":fixture.members[0].id,"organizationId":fixture.members[0].org});
        let csrf=text(&snapshot.body,"csrfToken")?.to_owned();
        let pending=tokio::spawn(async move {console(&pending_app,Method::POST,"/auth/context-token",Some(&pending_cookie),Some(&csrf),body).await});
        let waited=waiting_for_user_lock(&db.pool).await;
        if waited.is_err() {owner.rollback().await?;pending.abort();return waited;}
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(fixture.dual.id).execute(&mut *owner).await?;
        sqlx::query("UPDATE sso_sessions SET revoked_at=clock_timestamp() WHERE id=$1").bind(original_sid).execute(&mut *owner).await?;
        owner.commit().await?;
        let stale=tokio::time::timeout(Duration::from_secs(3),pending).await.map_err(|_|anyhow!("account generation waiter did not finish"))???;
        expect(&stale,StatusCode::UNAUTHORIZED,"late database lock must not adopt a fresh account generation or another SSO sid")?;
        let unchanged:(i64,Uuid,i64)=sqlx::query_as("SELECT b.account_version,b.global_session_id,(SELECT count(*) FROM platform_sessions s WHERE s.bff_session_id=b.id) FROM bff_sessions b WHERE b.id=$1")
            .bind(id).fetch_one(&db.pool).await?;
        ensure!(unchanged.0==0&&unchanged.1==original_sid&&unchanged.2==0,"failed original account proof partially committed or rebound platform context");
        let (_,fresh)=bff.login(&fixture,&fixture.dual).await?;
        let valid=selected(&bff,&fresh,fixture.members[0]).await?;
        expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(text(&valid.body,"accessToken")?),Value::Null).await?,StatusCode::OK,"new explicit authentication can establish the fresh account generation")?;
        Ok(())
    }.await;
    db.finish(result).await
}

async fn two_second_deadline(pool: &PgPool) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint+2")
            .fetch_one(pool)
            .await?,
    )
}

async fn wait_past_deadline(pool: &PgPool, expires: i64) -> Result<()> {
    let bound = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let expired: bool =
            sqlx::query_scalar("SELECT clock_timestamp()>=to_timestamp($1::double precision)")
                .bind(expires as f64)
                .fetch_one(pool)
                .await?;
        if expired {
            return Ok(());
        }
        ensure!(
            tokio::time::Instant::now() < bound,
            "owned deadline did not pass within its real clock bound"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn id_deadline(token: &str) -> Result<i64> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| anyhow!("real ID Token omitted its payload"))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload)?;
    let claims: Value = serde_json::from_slice(&bytes)?;
    claims["exp"]
        .as_i64()
        .ok_or_else(|| anyhow!("real ID Token omitted expiry"))
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_shorter_online_access_deadline_survives_network_and_account_lock_wait() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;let bff=Bff::new(&fixture)?;
        let (_,session)=bff.login(&fixture,&fixture.dual).await?;
        let original=selected(&bff,&session,fixture.members[0]).await?;
        let bearer=text(&original.body,"accessToken")?.to_owned();
        let id=bff_id(&db.pool,&session).await?;
        let saved:(i64,Option<Uuid>,i64)=sqlx::query_as("SELECT version,selected_session_id,(SELECT count(*) FROM platform_sessions p WHERE p.bff_session_id=b.id) FROM bff_sessions b WHERE b.id=$1")
            .bind(id).fetch_one(&db.pool).await?;

        // Only shorten a real introspection response; all subject, grant, RP,
        // account and signature evidence still comes from the actual provider.
        let deadline=two_second_deadline(&db.pool).await?;
        bff.transport.shorter_access_exp.store(deadline,Ordering::SeqCst);
        bff.transport.pause_userinfo.store(true,Ordering::SeqCst);
        let app=bff.app.clone();
        let pending=tokio::spawn(async move {call(&app,Method::GET,"/api/auth/profile",Some(&bearer),Value::Null).await});
        if tokio::time::timeout(Duration::from_secs(1),bff.transport.userinfo_entered.notified()).await.is_err() {
            pending.abort();return Err(anyhow!("CurrentUser did not verify the shorter live proof before UserInfo barrier"));
        }
        let waited=wait_past_deadline(&db.pool,deadline).await;
        bff.transport.userinfo_release.notify_one();
        let reply=tokio::time::timeout(Duration::from_secs(2),pending).await.map_err(|_|anyhow!("CurrentUser did not finish after deadline release"))???;
        waited?;
        expect(&reply,StatusCode::UNAUTHORIZED,"network wait cannot outlive the shorter original introspection deadline")?;

        bff.transport.shorter_access_exp.store(0,Ordering::SeqCst);
        let snapshot=console_session(&bff.app,&session).await?;
        expect(&snapshot,StatusCode::OK,"negative live deadline did not alter the stored real authority")?;
        let mut owner=db.pool.begin().await?;
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(fixture.dual.id).execute(&mut *owner).await?;
        let deadline=two_second_deadline(&db.pool).await?;
        bff.transport.shorter_access_exp.store(deadline,Ordering::SeqCst);
        bff.transport.pause_userinfo.store(true,Ordering::SeqCst);
        let app=bff.app.clone();let cookie=session.cookie.clone();let csrf=text(&snapshot.body,"csrfToken")?.to_owned();
        let body=json!({"expectedVersion":version(&snapshot)?,"membershipId":fixture.members[2].id,"organizationId":fixture.members[2].org});
        let pending=tokio::spawn(async move {console(&app,Method::POST,"/auth/context-token",Some(&cookie),Some(&csrf),body).await});
        if tokio::time::timeout(Duration::from_secs(1),bff.transport.userinfo_entered.notified()).await.is_err() {
            owner.rollback().await?;pending.abort();return Err(anyhow!("context did not verify the shorter live proof before UserInfo barrier"));
        }
        bff.transport.userinfo_release.notify_one();
        let waited=async {waiting_for_user_lock(&db.pool).await?;wait_past_deadline(&db.pool,deadline).await}.await;
        owner.rollback().await?;
        let reply=tokio::time::timeout(Duration::from_secs(2),pending).await.map_err(|_|anyhow!("context deadline waiter did not finish"))???;
        waited?;
        expect(&reply,StatusCode::UNAUTHORIZED,"account-lock wait cannot extend the shorter online access deadline")?;
        let after:(i64,Option<Uuid>,i64)=sqlx::query_as("SELECT version,selected_session_id,(SELECT count(*) FROM platform_sessions p WHERE p.bff_session_id=b.id) FROM bff_sessions b WHERE b.id=$1")
            .bind(id).fetch_one(&db.pool).await?;
        ensure!(after==saved,"expired online proof partially switched or minted a platform context");
        bff.transport.shorter_access_exp.store(0,Ordering::SeqCst);
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_real_short_id_expiry_cannot_commit_callback_or_rotated_refresh() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;let bff=Bff::new(&fixture)?;let provider=fixture.app();
        let sso=sso_login(&provider,&fixture.dual,PASSWORD).await?;
        let login=console_login(&bff.app,&fixture.employee,"/",None).await?;
        let (_,code)=provider_approval(&provider,&fixture.employee,&login,&sso,None).await?;
        bff.transport.fault(Fault::EarlyRealId);
        bff.transport.pause_userinfo.store(true,Ordering::SeqCst);
        let app=bff.app.clone();
        let pending=tokio::spawn(async move {callback(&app,&login,&code).await});
        if tokio::time::timeout(Duration::from_secs(1),bff.transport.userinfo_entered.notified()).await.is_err() {
            pending.abort();return Err(anyhow!("callback did not validate the real short-lived ID before its final UserInfo response"));
        }
        let observed=bff.transport.captured()?;
        let expires=id_deadline(&observed.id)?;
        let mut owner=db.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='250ms'").execute(&mut *owner).await?;
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(fixture.dual.id).execute(&mut *owner).await?;
        bff.transport.userinfo_release.notify_one();
        let waited=async {
            waiting_for_user_lock(&db.pool).await?;
            wait_past_deadline(&db.pool,expires).await?;
            expect(&call(&provider,Method::GET,"/oidc/userinfo",Some(&observed.access),Value::Null).await?,StatusCode::OK,"real fresh access remains valid after original ID expiration")?;
            Ok::<(),anyhow::Error>(())
        }.await;
        owner.rollback().await?;
        let reply=tokio::time::timeout(Duration::from_secs(2),pending).await.map_err(|_|anyhow!("callback ID-expiry waiter did not finish"))???;
        waited?;
        expect(&reply,StatusCode::UNAUTHORIZED,"expired original signed ID cannot commit a callback session")?;
        let callback_rows:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM bff_sessions WHERE user_id=$1),(SELECT count(*) FROM platform_sessions WHERE bff_session_id IS NOT NULL AND user_id=$1)").bind(fixture.dual.id)
            .fetch_one(&db.pool).await?;
        ensure!(callback_rows==(0,0),"ID expiry partially committed callback or platform session state");

        bff.transport.fault(Fault::Healthy);
        let (_,session)=bff.login(&fixture,&fixture.dual).await?;
        let _=selected(&bff,&session,fixture.members[0]).await?;
        let id=bff_id(&db.pool,&session).await?;
        let before:(String,String,i64,Option<Uuid>,chrono::DateTime<chrono::Utc>,i64)=sqlx::query_as("SELECT access_ciphertext,refresh_ciphertext,version,selected_session_id,expires_at,(SELECT count(*) FROM platform_sessions p WHERE p.bff_session_id=b.id) FROM bff_sessions b WHERE b.id=$1")
            .bind(id).fetch_one(&db.pool).await?;
        sqlx::query("UPDATE bff_sessions SET access_expires_at=clock_timestamp()+interval '10 seconds' WHERE id=$1").bind(id).execute(&db.pool).await?;
        bff.transport.fault(Fault::EarlyRealId);
        bff.transport.pause_userinfo.store(true,Ordering::SeqCst);
        let app=bff.app.clone();let cookie=session.cookie.clone();
        let pending=tokio::spawn(async move {console(&app,Method::GET,"/auth/session",Some(&cookie),None,Value::Null).await});
        if tokio::time::timeout(Duration::from_secs(1),bff.transport.userinfo_entered.notified()).await.is_err() {
            pending.abort();return Err(anyhow!("refresh did not validate the real short-lived ID before its final UserInfo response"));
        }
        let observed=bff.transport.captured()?;
        let expires=id_deadline(&observed.id)?;
        let mut owner=db.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='250ms'").execute(&mut *owner).await?;
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(fixture.dual.id).execute(&mut *owner).await?;
        bff.transport.userinfo_release.notify_one();
        let waited=async {
            waiting_for_user_lock(&db.pool).await?;
            wait_past_deadline(&db.pool,expires).await?;
            expect(&call(&provider,Method::GET,"/oidc/userinfo",Some(&observed.access),Value::Null).await?,StatusCode::OK,"real rotated access is independently live after original ID expiration")?;
            Ok::<(),anyhow::Error>(())
        }.await;
        owner.rollback().await?;
        let reply=tokio::time::timeout(Duration::from_secs(2),pending).await.map_err(|_|anyhow!("refresh ID-expiry waiter did not finish"))???;
        waited?;
        expect(&reply,StatusCode::UNAUTHORIZED,"expired original signed ID cannot commit a rotated BFF credential pair")?;
        let after:(String,String,i64,Option<Uuid>,chrono::DateTime<chrono::Utc>,i64)=sqlx::query_as("SELECT access_ciphertext,refresh_ciphertext,version,selected_session_id,expires_at,(SELECT count(*) FROM platform_sessions p WHERE p.bff_session_id=b.id) FROM bff_sessions b WHERE b.id=$1")
            .bind(id).fetch_one(&db.pool).await?;
        ensure!(after==before,"expired ID partially wrote rotated credentials, context or an extended absolute lifetime");
        let lease:Option<Uuid>=sqlx::query_scalar("SELECT refresh_lease_id FROM bff_sessions WHERE id=$1").bind(id).fetch_one(&db.pool).await?;
        ensure!(lease.is_some(),"failed post-send ID proof removed the durable no-replay refresh lease");
        bff.transport.fault(Fault::Healthy);
        let sends=bff.transport.refreshes.load(Ordering::SeqCst);
        expect(&console_session(&bff.other_instance(),&session).await?,StatusCode::SERVICE_UNAVAILABLE,"another instance cannot retry the refresh after a late ID failure")?;
        ensure!(bff.transport.refreshes.load(Ordering::SeqCst)==sends,"late ID failure replayed an already-sent refresh");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_cookie_reload_recovers_only_original_unknown_logout_without_repeating_effects(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;let bff=Bff::new(&fixture)?;
        for scope in ["PLATFORM","GLOBAL"] {
            let (sso,session)=bff.login(&fixture,&fixture.dual).await?;
            let context=selected(&bff,&session,fixture.members[0]).await?;
            let token=text(&context.body,"accessToken")?.to_owned();
            let snapshot=console_session(&bff.app,&session).await?;
            let id=bff_id(&db.pool,&session).await?;
            let generation:i64=sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1").bind(fixture.dual.id).fetch_one(&db.pool).await?;
            let before=bff.transport.logout_attempts()?.len();
            bff.transport.fault(Fault::LogoutResponseLost);
            let lost=console(&bff.app,Method::POST,"/auth/logout",Some(&session.cookie),Some(text(&snapshot.body,"csrfToken")?),
                json!({"expectedVersion":version(&snapshot)?,"scope":scope})).await?;
            expect(&lost,StatusCode::SERVICE_UNAVAILABLE,"actual committed provider logout with lost response is unknown")?;
            no_cookie_mutation(&lost)?;
            let pending:(Uuid,String,i64,Uuid)=sqlx::query_as("SELECT logout_request_id,logout_scope,version,oidc_grant_id FROM bff_sessions WHERE id=$1 AND revoked_at IS NULL")
                .bind(id).fetch_one(&db.pool).await?;
            ensure!(pending.1==scope&&pending.2.to_string()==version(&snapshot)?,"unknown logout changed its durable command scope or version");
            let revoked_at:chrono::DateTime<chrono::Utc>=sqlx::query_scalar("SELECT revoked_at FROM oidc_grants WHERE id=$1").bind(pending.3).fetch_one(&db.pool).await?;
            let first=bff.transport.logout_attempts()?;
            ensure!(first.len()==before+1&&first[before].endpoint==endpoint_index(if scope=="GLOBAL"{crate::bff::Endpoint::GlobalLogout}else{crate::bff::Endpoint::Revoke})
                &&first[before].request_id==if scope=="GLOBAL"{Some(pending.0)}else{None},"unknown fixture did not execute the exact original provider logout command");
            let counts=(bff.transport.count(crate::bff::Endpoint::Revoke),bff.transport.count(crate::bff::Endpoint::GlobalLogout));
            let wrong_origin=exchange(&bff.app,Method::GET,"/auth/session",None,String::new(),&[
                ("host","console.example.invalid"),("origin","https://attacker.example.invalid"),("cookie",&session.cookie),
            ]).await?;
            expect(&wrong_origin,StatusCode::FORBIDDEN,"cross-site GET cannot resume a pending logout")?;
            no_store(&wrong_origin)?;no_cookie_mutation(&wrong_origin)?;
            let opposite=if scope=="GLOBAL"{"PLATFORM"}else{"GLOBAL"};
            let switched=console(&bff.app,Method::POST,"/auth/logout",Some(&session.cookie),Some(text(&snapshot.body,"csrfToken")?),
                json!({"expectedVersion":version(&snapshot)?,"scope":opposite})).await?;
            expect(&switched,StatusCode::CONFLICT,"pending logout scope cannot be replaced by a retry")?;
            no_cookie_mutation(&switched)?;
            let injected=console(&bff.app,Method::GET,&format!("/auth/session?scope={opposite}"),Some(&session.cookie),None,Value::Null).await?;
            expect(&injected,StatusCode::BAD_REQUEST,"recovery GET accepts no new logout scope from its URL")?;
            no_cookie_mutation(&injected)?;
            ensure!(counts==(bff.transport.count(crate::bff::Endpoint::Revoke),bff.transport.count(crate::bff::Endpoint::GlobalLogout)),
                "rejected Origin or scope caused an upstream logout attempt");
            bff.transport.fault(Fault::Healthy);
            // A new page has only its HttpOnly Cookie and browser fetch metadata;
            // neither memory CSRF nor an in-memory logout scope survives reload.
            let recovered=cookie_reload(&bff.other_instance(),&session).await?;
            expect(&recovered,StatusCode::UNAUTHORIZED,"same-origin Cookie reload confirms original pending logout and returns anonymous")?;
            no_cookie_mutation(&recovered)?;
            let attempts=bff.transport.logout_attempts()?;
            ensure!(attempts.len()==before+2&&attempts[before]==attempts[before+1],
                "reload changed the original logout token, endpoint, or idempotency request ID");
            let closed:(Uuid,String,bool)=sqlx::query_as("SELECT logout_request_id,logout_scope,revoked_at IS NOT NULL FROM bff_sessions WHERE id=$1")
                .bind(id).fetch_one(&db.pool).await?;
            ensure!(closed.0==pending.0&&closed.1==scope&&closed.2,"recovery did not close exactly the original durable logout");
            let current_generation:i64=sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1").bind(fixture.dual.id).fetch_one(&db.pool).await?;
            ensure!(current_generation==generation+if scope=="GLOBAL"{1}else{0},"logout recovery repeated or broadened account-generation revocation");
            let after_revocation:chrono::DateTime<chrono::Utc>=sqlx::query_scalar("SELECT revoked_at FROM oidc_grants WHERE id=$1").bind(pending.3).fetch_one(&db.pool).await?;
            ensure!(after_revocation==revoked_at,"idempotent logout recovery repeated the provider's revocation effect");
            if scope=="GLOBAL" {
                let once:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM oidc_account_logout_receipts WHERE id=$1),(SELECT count(*) FROM audit_logs WHERE action='OIDC_ACCOUNT_LOGOUT' AND detail::jsonb->>'requestId'=$2)")
                    .bind(pending.0).bind(pending.0.to_string()).fetch_one(&db.pool).await?;
                ensure!(once==(1,1),"unknown global logout was applied more than once");
            }
            let sso_status=if scope=="GLOBAL"{StatusCode::UNAUTHORIZED}else{StatusCode::OK};
            expect(&sso_session(&fixture.app(),&sso).await?,sso_status,"recovered logout preserves its exact global versus platform scope")?;
            expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(&token),Value::Null).await?,StatusCode::UNAUTHORIZED,"original context is revoked after recovered logout")?;
            let repeated=cookie_reload(&bff.app,&session).await?;
            expect(&repeated,StatusCode::UNAUTHORIZED,"retained old opaque Cookie has no authority after recovery")?;
            no_cookie_mutation(&repeated)?;
            ensure!(bff.transport.logout_attempts()?.len()==before+2,"anonymous old Cookie replay sent another logout command");
        }
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires an explicitly configured, disposable Phase 10 PostgreSQL server"]
async fn bff_late_logout_and_delayed_success_cannot_erase_a_new_account_cookie() -> Result<()> {
    let db = Database::create().await?;
    let result = async {
        let fixture = Fixture::create(&db.pool).await?;
        let bff = Bff::new(&fixture)?;
        let provider = fixture.app();
        let old_sso = sso_login(&provider, &fixture.dual, PASSWORD).await?;
        let first_login = console_login(&bff.app, &fixture.employee, "/", None).await?;
        let (provider_flow, code) =
            provider_approval(&provider, &fixture.employee, &first_login, &old_sso, None).await?;
        let old = completed_login(&bff.app, &first_login, &code).await?;
        let original = selected(&bff, &old, fixture.members[0]).await?;
        let old_token = text(&original.body, "accessToken")?.to_owned();
        let snapshot = console_session(&bff.app, &old).await?;
        bff.transport.pause_logout.store(true, Ordering::SeqCst);
        let app = bff.app.clone();
        let cookie = old.cookie.clone();
        let csrf = text(&snapshot.body, "csrfToken")?.to_owned();
        let body = json!({"expectedVersion":version(&snapshot)?,"scope":"GLOBAL"});
        let pending = tokio::spawn(async move {
            console(
                &app,
                Method::POST,
                "/auth/logout",
                Some(&cookie),
                Some(&csrf),
                body,
            )
            .await
        });
        if tokio::time::timeout(
            Duration::from_secs(1),
            bff.transport.logout_entered.notified(),
        )
        .await
        .is_err()
        {
            pending.abort();
            return Err(anyhow!(
                "logout did not reach its actual committed-provider response barrier"
            ));
        }
        let replacement = async {
            let new_sso = sso_login(&provider, &fixture.memberless, PASSWORD).await?;
            let new_login =
                console_login(&bff.app, &fixture.employee, "/", Some(&first_login.cookie)).await?;
            let (_, code) = provider_approval(
                &provider,
                &fixture.employee,
                &new_login,
                &new_sso,
                Some(&provider_flow.cookie),
            )
            .await?;
            completed_login(&bff.app, &new_login, &code).await
        }
        .await;
        bff.transport.logout_release.notify_one();
        let late = tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .map_err(|_| anyhow!("late logout did not finish after response release"))???;
        let replacement = replacement?;
        expect(
            &late,
            StatusCode::CONFLICT,
            "old pending logout cannot finalize against another tab's newly committed account",
        )?;
        no_cookie_mutation(&late)?;
        let new_state = console_session(&bff.app, &replacement).await?;
        expect(
            &new_state,
            StatusCode::OK,
            "new different-account Cookie remains usable after old logout response",
        )?;
        ensure!(
            new_state.body["account"]["id"] == json!(fixture.memberless.id)
                && replacement.cookie != old.cookie,
            "late logout replaced or rebound the newer browser account"
        );
        let old_read = console_session(&bff.app, &old).await?;
        expect(
            &old_read,
            StatusCode::UNAUTHORIZED,
            "old account Cookie remains retired after replacement callback",
        )?;
        no_cookie_mutation(&old_read)?;
        expect(
            &call(
                &bff.app,
                Method::GET,
                "/api/auth/profile",
                Some(&old_token),
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
            "late logout cannot revive the old account context",
        )?;

        // Hold a fully successful response at the client boundary. The server
        // has committed logout before another login, but delivery may reverse.
        let delayed_success = console(
            &bff.app,
            Method::POST,
            "/auth/logout",
            Some(&replacement.cookie),
            Some(text(&new_state.body, "csrfToken")?),
            json!({"expectedVersion":version(&new_state)?,"scope":"PLATFORM"}),
        )
        .await?;
        expect(
            &delayed_success,
            StatusCode::OK,
            "server logout can finish before its response is delivered",
        )?;
        no_cookie_mutation(&delayed_success)?;
        let final_sso = sso_login(&provider, &fixture.dual, PASSWORD).await?;
        let final_login =
            console_login(&bff.app, &fixture.employee, "/", Some(&first_login.cookie)).await?;
        let (_, code) = provider_approval(
            &provider,
            &fixture.employee,
            &final_login,
            &final_sso,
            Some(&provider_flow.cookie),
        )
        .await?;
        let final_session = completed_login(&bff.app, &final_login, &code).await?;
        // Applying the withheld old HTTP response has no cookie operation at
        // all, so it cannot delete the freshly installed fixed-name value.
        no_cookie_mutation(&delayed_success)?;
        let final_state = console_session(&bff.app, &final_session).await?;
        expect(
            &final_state,
            StatusCode::OK,
            "new login survives delivery of a previously successful logout response",
        )?;
        ensure!(
            final_state.body["account"]["id"] == json!(fixture.dual.id)
                && final_session.cookie != replacement.cookie,
            "delayed logout response erased or substituted a new account Cookie"
        );
        let retired = console_session(&bff.app, &replacement).await?;
        expect(
            &retired,
            StatusCode::UNAUTHORIZED,
            "successfully logged-out older Cookie stays unauthorized",
        )?;
        no_cookie_mutation(&retired)?;
        Ok(())
    }
    .await;
    db.finish(result).await
}
