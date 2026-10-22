//! Same-origin map resources. Upstream hosts are fixed, so this cannot be used
//! as a general URL proxy. The gateway may supply a trusted client IP via the
//! normal normalize_client_ip middleware before the IP location handler runs.
use std::net::IpAddr;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/maps/tiles/{z}/{x}/{file}", get(tile))
        .route("/api/maps/search", get(search))
        .route("/api/maps/reverse", get(reverse))
        .route("/api/maps/ip-location", get(ip_location))
}

fn append_bounded(body: &mut Vec<u8>, chunk: &[u8], limit: usize) -> Result<(), ()> {
    if chunk.len() > limit.saturating_sub(body.len()) {
        return Err(());
    }
    body.extend_from_slice(chunk);
    Ok(())
}

async fn bounded_body(
    mut response: reqwest::Response,
    limit: usize,
    invalid: &'static str,
    too_large: &'static str,
) -> Result<Vec<u8>, AppError> {
    if response
        .content_length()
        .is_some_and(|len| len > limit as u64)
    {
        return Err(AppError::Server(anyhow::anyhow!(too_large)));
    }
    // Content-Length may be absent or inaccurate. Stop at the first chunk that
    // exceeds the budget instead of collecting an unbounded upstream response.
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::Server(anyhow::anyhow!(invalid)))?
    {
        append_bounded(&mut body, &chunk, limit)
            .map_err(|_| AppError::Server(anyhow::anyhow!(too_large)))?;
    }
    Ok(body)
}

fn valid_tile(z: u8, x: u32, file: &str) -> Option<u32> {
    let y: u32 = file.strip_suffix(".png")?.parse().ok()?;
    let dimension = 1u64.checked_shl(u32::from(z))?;
    (z <= 19 && u64::from(x) < dimension && u64::from(y) < dimension).then_some(y)
}

async fn tile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((z, x, file)): Path<(u8, u32, String)>,
) -> Result<Response, AppError> {
    state.check_map_rate(
        &format!("tile:{}", crate::util::audit::client_ip(&headers)),
        300,
    )?;
    let y = valid_tile(z, x, &file).ok_or_else(|| AppError::bad_request("无效的地图瓦片坐标"))?;
    let url = format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png");
    let response = state
        .map_client
        .get(url)
        .send()
        .await
        .map_err(|_| AppError::Server(anyhow::anyhow!("MAP_TILE_UPSTREAM_UNAVAILABLE")))?;
    if !response.status().is_success()
        || response.content_length().is_some_and(|len| len > 2_000_000)
    {
        return Err(AppError::Server(anyhow::anyhow!(
            "MAP_TILE_UPSTREAM_FAILED"
        )));
    }
    let bytes = bounded_body(
        response,
        2_000_000,
        "MAP_TILE_UPSTREAM_FAILED",
        "MAP_TILE_TOO_LARGE",
    )
    .await?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Deserialize)]
struct ReverseQuery {
    lat: f64,
    lon: f64,
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    state.check_map_rate(
        &format!("lookup:{}", crate::util::audit::client_ip(&headers)),
        20,
    )?;
    let keyword = query.q.trim();
    if keyword.is_empty() || keyword.chars().count() > 120 {
        return Err(AppError::bad_request("地点关键词需为 1～120 个字符"));
    }
    let response = state
        .map_client
        .get("https://nominatim.openstreetmap.org/search")
        .query(&[
            ("format", "jsonv2"),
            ("limit", "5"),
            ("addressdetails", "1"),
            ("q", keyword),
        ])
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9")
        .send()
        .await
        .map_err(|_| AppError::Server(anyhow::anyhow!("MAP_SEARCH_UPSTREAM_UNAVAILABLE")))?;
    if !response.status().is_success() {
        return Err(AppError::Server(anyhow::anyhow!(
            "MAP_SEARCH_UPSTREAM_FAILED"
        )));
    }
    let bytes = bounded_body(
        response,
        200_000,
        "MAP_SEARCH_INVALID_RESPONSE",
        "MAP_SEARCH_RESPONSE_TOO_LARGE",
    )
    .await?;
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Server(anyhow::anyhow!("MAP_SEARCH_INVALID_RESPONSE")))?;
    if !data.is_array() {
        return Err(AppError::Server(anyhow::anyhow!(
            "MAP_SEARCH_INVALID_RESPONSE"
        )));
    }
    Ok(Json(data))
}

async fn reverse(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReverseQuery>,
) -> Result<Json<Value>, AppError> {
    state.check_map_rate(
        &format!("lookup:{}", crate::util::audit::client_ip(&headers)),
        20,
    )?;
    if !query.lat.is_finite()
        || !query.lon.is_finite()
        || !(-90.0..=90.0).contains(&query.lat)
        || !(-180.0..=180.0).contains(&query.lon)
    {
        return Err(AppError::bad_request("无效的经纬度"));
    }
    let response = state
        .map_client
        .get("https://nominatim.openstreetmap.org/reverse")
        .query(&[
            ("format", "jsonv2"),
            ("addressdetails", "1"),
            ("zoom", "18"),
        ])
        .query(&[("lat", query.lat), ("lon", query.lon)])
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9")
        .send()
        .await
        .map_err(|_| AppError::Server(anyhow::anyhow!("MAP_REVERSE_UPSTREAM_UNAVAILABLE")))?;
    if !response.status().is_success() {
        return Err(AppError::Server(anyhow::anyhow!(
            "MAP_REVERSE_UPSTREAM_FAILED"
        )));
    }
    let bytes = bounded_body(
        response,
        100_000,
        "MAP_REVERSE_INVALID_RESPONSE",
        "MAP_REVERSE_RESPONSE_TOO_LARGE",
    )
    .await?;
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Server(anyhow::anyhow!("MAP_REVERSE_INVALID_RESPONSE")))?;
    if !data.is_object() {
        return Err(AppError::Server(anyhow::anyhow!(
            "MAP_REVERSE_INVALID_RESPONSE"
        )));
    }
    Ok(Json(data))
}

fn public_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let raw = headers.get("x-real-ip")?.to_str().ok()?;
    let ip: IpAddr = raw.parse().ok()?;
    match ip {
        IpAddr::V4(v4)
            if v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified() =>
        {
            None
        }
        IpAddr::V6(v6)
            if v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
                || v6.is_unspecified() =>
        {
            None
        }
        _ => Some(ip),
    }
}

async fn ip_location(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.check_map_rate(
        &format!("lookup:{}", crate::util::audit::client_ip(&headers)),
        20,
    )?;
    let ip = public_ip(&headers)
        .ok_or_else(|| AppError::bad_request("无法识别访问者公网 IP，请使用设备定位或地图选点"))?;
    let response = state
        .map_client
        .get(format!("https://ipwho.is/{ip}"))
        .send()
        .await
        .map_err(|_| AppError::Server(anyhow::anyhow!("IP_LOCATION_UPSTREAM_UNAVAILABLE")))?;
    if !response.status().is_success() {
        return Err(AppError::Server(anyhow::anyhow!(
            "IP_LOCATION_UPSTREAM_FAILED"
        )));
    }
    let bytes = bounded_body(
        response,
        100_000,
        "IP_LOCATION_INVALID_RESPONSE",
        "IP_LOCATION_RESPONSE_TOO_LARGE",
    )
    .await?;
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Server(anyhow::anyhow!("IP_LOCATION_INVALID_RESPONSE")))?;
    if data["success"] != true {
        return Err(AppError::bad_request("当前公网 IP 无法估算位置"));
    }
    Ok(Json(json!({
        "success": true,
        "latitude": data["latitude"],
        "longitude": data["longitude"],
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_byte_budget_stops_before_retaining_an_oversized_chunk() {
        let mut bytes = Vec::new();
        append_bounded(&mut bytes, b"ab", 5).unwrap();
        append_bounded(&mut bytes, b"cde", 5).unwrap();
        assert_eq!(bytes, b"abcde");
        assert!(append_bounded(&mut bytes, b"f", 5).is_err());
        assert_eq!(bytes, b"abcde");
        assert!(append_bounded(&mut bytes, b"", 5).is_ok());
        let mut small = vec![1];
        assert!(append_bounded(&mut small, &[2, 3, 4, 5, 6], 5).is_err());
        assert_eq!(small, [1]);
    }

    #[tokio::test]
    async fn upstream_body_accepts_the_limit_and_rejects_known_oversize() {
        let response =
            |body: &'static str| reqwest::Response::from(axum::http::Response::new(body));
        assert_eq!(
            bounded_body(response("hello"), 5, "INVALID", "TOO_LARGE")
                .await
                .unwrap(),
            b"hello"
        );
        assert!(
            matches!(bounded_body(response("hello!"), 5, "INVALID", "TOO_LARGE")
            .await, Err(AppError::Server(error)) if error.to_string()=="TOO_LARGE")
        );
        assert!(bounded_body(response(""), 0, "INVALID", "TOO_LARGE")
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn tile_path_and_client_ip_are_bounded() {
        assert_eq!(valid_tile(4, 3, "7.png"), Some(7));
        assert_eq!(valid_tile(4, 3, "16.png"), None);
        assert_eq!(valid_tile(20, 3, "7.png"), None);
        assert_eq!(valid_tile(4, 3, "7.svg"), None);
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "127.0.0.1".parse().unwrap());
        assert_eq!(public_ip(&headers), None);
        headers.insert("x-real-ip", "8.8.8.8".parse().unwrap());
        assert_eq!(public_ip(&headers), Some("8.8.8.8".parse().unwrap()));
    }
}
