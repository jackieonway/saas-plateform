//! Resolve one bounded source identity before headers reach handlers or audits.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderMap, HeaderValue};

// Parsing work stays bounded even when a configured proxy passes an oversized
// chain. A malformed chain never falls back to another caller-supplied header.
const MAX_FORWARDED_BYTES: usize = 2048;
const MAX_FORWARDED_HOPS: usize = 32;

fn trusted(ip: IpAddr, proxies: &[IpAddr]) -> bool {
    proxies
        .iter()
        .any(|proxy| proxy.to_canonical() == ip.to_canonical())
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?;
    if values.next().is_some() || value.as_bytes().len() > MAX_FORWARDED_BYTES {
        return None;
    }
    value.to_str().ok()
}

fn forwarded_identity(headers: &HeaderMap, peer: IpAddr, proxies: &[IpAddr]) -> Option<IpAddr> {
    if headers.contains_key("x-forwarded-for") {
        let value = single_header(headers, "x-forwarded-for")?;
        let mut chain = Vec::new();
        for raw in value.split(',') {
            if chain.len() == MAX_FORWARDED_HOPS {
                return None;
            }
            chain.push(raw.trim().parse::<IpAddr>().ok()?.to_canonical());
        }
        // Start at the actual connection and stop at the nearest untrusted hop.
        // Entries to its left may have been supplied by that untrusted caller.
        let mut identity = peer;
        for hop in chain.into_iter().rev() {
            if !trusted(identity, proxies) {
                break;
            }
            identity = hop;
        }
        Some(identity)
    } else {
        // Single-hop gateways may send only the header they overwrite. This is
        // accepted solely for the configured, actual socket peer.
        single_header(headers, "x-real-ip")?
            .trim()
            .parse::<IpAddr>()
            .ok()
            .map(|ip| ip.to_canonical())
    }
}

pub(super) fn normalize(request: &mut Request, proxies: &[IpAddr]) {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|peer| peer.0.ip().to_canonical());
    let identity = peer
        .map(|peer| {
            if trusted(peer, proxies) {
                forwarded_identity(request.headers(), peer, proxies).unwrap_or(peer)
            } else {
                peer
            }
        })
        .map(|ip| ip.to_string())
        // In-process requests without a socket share a fail-closed source bucket.
        .unwrap_or_else(|| "unknown".into());
    let headers = request.headers_mut();
    headers.remove("x-real-ip");
    headers.remove("x-forwarded-for");
    headers.remove("forwarded");
    headers.insert(
        "x-real-ip",
        HeaderValue::from_str(&identity).expect("canonical IP or fixed unknown"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::middleware::Next;
    use axum::response::Response;
    use axum::{routing::get, Router};
    use tower::ServiceExt;

    fn resolve(peer: Option<&str>, proxies: &[&str], headers: &[(&str, &str)]) -> String {
        let mut request = Request::new(Body::empty());
        if let Some(peer) = peer {
            request
                .extensions_mut()
                .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        }
        for (name, value) in headers {
            request.headers_mut().append(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        normalize(
            &mut request,
            &proxies
                .iter()
                .map(|ip| ip.parse().unwrap())
                .collect::<Vec<_>>(),
        );
        assert!(!request.headers().contains_key("x-forwarded-for"));
        assert!(!request.headers().contains_key("forwarded"));
        crate::util::audit::client_ip(request.headers())
    }

    #[test]
    fn direct_socket_identity_ignores_all_self_reported_headers() {
        let forged = [
            ("x-real-ip", "198.51.100.99"),
            ("x-forwarded-for", "198.51.100.98, 192.0.2.1"),
            ("forwarded", "for=198.51.100.97"),
        ];
        assert_eq!(
            resolve(Some("203.0.113.7:3123"), &[], &forged),
            "203.0.113.7"
        );
        assert_eq!(
            resolve(Some("[2001:db8::7]:3123"), &["192.0.2.1"], &forged),
            "2001:db8::7"
        );
        assert_eq!(resolve(None, &["192.0.2.1"], &forged), "unknown");
    }

    #[test]
    fn trusted_proxy_uses_controlled_chain_before_real_ip() {
        let headers = [
            ("x-real-ip", "198.51.100.99"),
            ("x-forwarded-for", "203.0.113.7"),
        ];
        assert_eq!(
            resolve(Some("192.0.2.1:3123"), &["192.0.2.1"], &headers),
            "203.0.113.7"
        );
        assert_eq!(
            resolve(
                Some("192.0.2.1:3123"),
                &["192.0.2.1"],
                &[("x-real-ip", "2001:db8::7")],
            ),
            "2001:db8::7"
        );
    }

    #[test]
    fn multi_hop_chain_stops_before_client_injected_prefix() {
        assert_eq!(
            resolve(
                Some("192.0.2.1:3123"),
                &["192.0.2.1", "192.0.2.2"],
                &[("x-forwarded-for", "198.51.100.99, 203.0.113.7, 192.0.2.2")],
            ),
            "203.0.113.7"
        );
        assert_eq!(
            resolve(
                Some("[2001:db8::1]:3123"),
                &["2001:db8::1", "2001:db8::2"],
                &[("x-forwarded-for", "198.51.100.99, 2001:db8::7, 2001:db8::2")],
            ),
            "2001:db8::7"
        );
        assert_eq!(
            resolve(
                Some("[::ffff:192.0.2.1]:3123"),
                &["192.0.2.1"],
                &[("x-forwarded-for", "::ffff:203.0.113.7")],
            ),
            "203.0.113.7"
        );
    }

    #[test]
    fn malformed_ambiguous_or_oversized_chain_falls_back_to_socket() {
        for raw in [
            "",
            "unknown",
            "garbage, 203.0.113.7",
            "203.0.113.7, ",
            "203.0.113.7:443",
            "[2001:db8::7]",
            &"203.0.113.7,".repeat(MAX_FORWARDED_HOPS + 1),
            &" ".repeat(MAX_FORWARDED_BYTES + 1),
        ] {
            assert_eq!(
                resolve(
                    Some("192.0.2.1:3123"),
                    &["192.0.2.1"],
                    &[("x-real-ip", "198.51.100.99"), ("x-forwarded-for", raw)],
                ),
                "192.0.2.1"
            );
        }
        assert_eq!(
            resolve(
                Some("192.0.2.1:3123"),
                &["192.0.2.1"],
                &[
                    ("x-forwarded-for", "203.0.113.7"),
                    ("x-forwarded-for", "203.0.113.8"),
                ],
            ),
            "192.0.2.1"
        );
        assert_eq!(
            resolve(Some("192.0.2.1:3123"), &["192.0.2.1"], &[]),
            "192.0.2.1"
        );
        for headers in [
            vec![("x-real-ip", "203.0.113.7, 203.0.113.8")],
            vec![("x-real-ip", "203.0.113.7"), ("x-real-ip", "203.0.113.8")],
            vec![("x-real-ip", "unknown")],
        ] {
            assert_eq!(
                resolve(Some("192.0.2.1:3123"), &["192.0.2.1"], &headers),
                "192.0.2.1"
            );
        }
    }

    #[test]
    fn non_text_forwarding_headers_cannot_choose_an_identity() {
        for header in ["x-forwarded-for", "x-real-ip"] {
            let mut request = Request::new(Body::empty());
            request
                .extensions_mut()
                .insert(ConnectInfo("192.0.2.1:3123".parse::<SocketAddr>().unwrap()));
            request
                .headers_mut()
                .insert(header, HeaderValue::from_bytes(&[0xff]).unwrap());
            normalize(&mut request, &["192.0.2.1".parse().unwrap()]);
            assert_eq!(
                crate::util::audit::client_ip(request.headers()),
                "192.0.2.1"
            );
            assert!(!request.headers().contains_key("x-forwarded-for"));
        }
    }

    async fn test_middleware(
        State(proxies): State<Vec<IpAddr>>,
        mut request: Request,
        next: Next,
    ) -> Response {
        normalize(&mut request, &proxies);
        next.run(request).await
    }

    #[tokio::test]
    async fn handlers_and_audits_receive_only_canonical_identity() {
        let router = Router::new()
            .route(
                "/",
                get(|headers: HeaderMap| async move {
                    assert!(!headers.contains_key("forwarded"));
                    assert!(!headers.contains_key("x-forwarded-for"));
                    crate::util::audit::client_ip(&headers)
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                vec!["192.0.2.1".parse::<IpAddr>().unwrap()],
                test_middleware,
            ));
        for (peer, expected) in [
            (Some("192.0.2.1:3123"), "203.0.113.7"),
            (Some("203.0.113.8:3123"), "203.0.113.8"),
            (None, "unknown"),
        ] {
            let mut request = Request::builder()
                .uri("/")
                .header("x-real-ip", "198.51.100.99")
                .header("x-forwarded-for", "198.51.100.98, 203.0.113.7")
                .header("forwarded", "for=198.51.100.97")
                .body(Body::empty())
                .unwrap();
            if let Some(peer) = peer {
                request
                    .extensions_mut()
                    .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
            }
            let response = router.clone().oneshot(request).await.unwrap();
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            assert_eq!(body.as_ref(), expected.as_bytes());
        }
    }
}
