//! Host navigation may use the SPA index; missing application resources retain 404.
use std::sync::Arc;

use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, MethodRouter};
use tower_http::services::ServeDir;

pub fn service(directory: &str) -> ServeDir<MethodRouter> {
    let index = Arc::new(std::path::Path::new(directory).join("index.html"));
    let fallback = get(move |request: Request| {
        let index = index.clone();
        async move { navigation_fallback(request, &index).await }
    });
    ServeDir::new(directory)
        .append_index_html_on_directories(true)
        .fallback(fallback)
}

async fn navigation_fallback(request: Request, index: &std::path::Path) -> Response {
    let Ok(path) = percent_encoding::percent_decode_str(request.uri().path()).decode_utf8() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let reserved = [
        "/api",
        "/oauth",
        "/sub",
        "/sub-apps",
        "/micro-assets",
        "/micro-proxy",
        "/micro-releases",
        "/releases",
    ]
    .iter()
    .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")));
    let filename = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let wants_html = request
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| {
            accept.split(',').any(|part| {
                let mut parts = part.split(';');
                parts
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("text/html")
                    && !parts.any(|parameter| {
                        parameter
                            .trim()
                            .strip_prefix("q=")
                            .and_then(|quality| quality.parse::<f32>().ok())
                            .is_some_and(|quality| quality <= 0.0)
                    })
            })
        });
    if reserved || filename.contains('.') || !wants_html {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(index).await {
        Ok(body) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            body,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use tower::ServiceExt;

    #[tokio::test]
    async fn host_navigation_uses_spa_and_missing_assets_remain_404() -> anyhow::Result<()> {
        let target =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/static-file-tests");
        tokio::fs::create_dir_all(&target).await?;
        let directory = target.join(uuid::Uuid::new_v4().simple().to_string());
        tokio::fs::create_dir(&directory).await?;
        tokio::fs::write(
            directory.join("index.html"),
            b"<!doctype html><div>host</div>",
        )
        .await?;
        tokio::fs::write(directory.join("app.css"), b"body{color:blue}").await?;
        let result: anyhow::Result<()> = async {
            let router = axum::Router::new().fallback_service(service(directory.to_str().unwrap()));
            for (path, accept, expected) in [
                ("/system/applications", "text/html", StatusCode::OK),
                ("/micro/vue-workshop/security", "text/html", StatusCode::OK),
                ("/system/applications", "*/*", StatusCode::NOT_FOUND),
                (
                    "/system/applications",
                    "text/html;q=0",
                    StatusCode::NOT_FOUND,
                ),
                ("/missing.js", "text/html", StatusCode::NOT_FOUND),
                ("/missing%2ejs", "text/html", StatusCode::NOT_FOUND),
                ("/assets/missing.css", "*/*", StatusCode::NOT_FOUND),
                ("/api/missing", "text/html", StatusCode::NOT_FOUND),
                ("/%61pi/missing", "text/html", StatusCode::NOT_FOUND),
                ("/oauth/missing", "text/html", StatusCode::NOT_FOUND),
                ("/sub-apps/demo/v1/", "text/html", StatusCode::NOT_FOUND),
                (
                    "/micro-proxy/demo/unknown/missing",
                    "text/html",
                    StatusCode::NOT_FOUND,
                ),
                (
                    "/micro-assets/demo/v1/missing",
                    "text/html",
                    StatusCode::NOT_FOUND,
                ),
            ] {
                let response = router
                    .clone()
                    .oneshot(
                        axum::http::Request::builder()
                            .uri(path)
                            .header(header::ACCEPT, accept)
                            .body(Body::empty())?,
                    )
                    .await?;
                assert_eq!(response.status(), expected, "{path} Accept {accept}");
                if expected == StatusCode::OK {
                    assert!(response.headers()[header::CONTENT_TYPE]
                        .to_str()?
                        .starts_with("text/html"));
                    assert!(String::from_utf8(
                        to_bytes(response.into_body(), 1024).await?.to_vec()
                    )?
                    .contains("host"));
                }
            }
            let response = router
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/app.css")
                        .body(Body::empty())?,
                )
                .await?;
            assert_eq!(response.status(), StatusCode::OK);
            assert!(response.headers()[header::CONTENT_TYPE]
                .to_str()?
                .starts_with("text/css"));
            Ok(())
        }
        .await;
        let resolved = tokio::fs::canonicalize(&directory).await?;
        let root = tokio::fs::canonicalize(&target).await?;
        anyhow::ensure!(
            resolved.starts_with(&root) && resolved != root,
            "unexpected cleanup target"
        );
        tokio::fs::remove_dir_all(&resolved).await?;
        result
    }
}
