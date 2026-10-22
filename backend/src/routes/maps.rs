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
    let bytes = response
        .bytes()
        .await
        .map_err(|_| AppError::Server(anyhow::anyhow!("MAP_TILE_UPSTREAM_FAILED")))?;
    if bytes.len() > 2_000_000 {
        return Err(AppError::Server(anyhow::anyhow!("MAP_TILE_TOO_LARGE")));
    }
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
    let bytes = response
        .bytes()
        .await
        .map_err(|_| AppError::Server(anyhow::anyhow!("MAP_SEARCH_INVALID_RESPONSE")))?;
    if bytes.len() > 200_000 {
        return Err(AppError::Server(anyhow::anyhow!(
            "MAP_SEARCH_RESPONSE_TOO_LARGE"
        )));
    }
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
    let bytes = response
        .bytes()
        .await
        .map_err(|_| AppError::Server(anyhow::anyhow!("MAP_REVERSE_INVALID_RESPONSE")))?;
    if bytes.len() > 100_000 {
        return Err(AppError::Server(anyhow::anyhow!(
            "MAP_REVERSE_RESPONSE_TOO_LARGE"
        )));
    }
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
    let bytes = response
        .bytes()
        .await
        .map_err(|_| AppError::Server(anyhow::anyhow!("IP_LOCATION_INVALID_RESPONSE")))?;
    if bytes.len() > 100_000 {
        return Err(AppError::Server(anyhow::anyhow!(
            "IP_LOCATION_RESPONSE_TOO_LARGE"
        )));
    }
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
