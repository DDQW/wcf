//! HTTP helpers shared by the custom (non-Leptos) routes.

use std::net::SocketAddr;

use axum::http::{header, HeaderMap, HeaderName, HeaderValue};
use tower_http::set_header::SetResponseHeaderLayer;

/// Baseline security headers for every response (only set if a handler did
/// not already set them, so file downloads can use stricter values).
pub fn security_headers() -> Vec<SetResponseHeaderLayer<HeaderValue>> {
    vec![
        SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
        SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ),
        SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("SAMEORIGIN"),
        ),
        SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static(
                "camera=(self), microphone=(self), display-capture=(self), geolocation=()",
            ),
        ),
    ]
}

/// Best-effort client IP: `X-Forwarded-For` (first hop) when running behind
/// a trusted proxy, otherwise the socket peer address.
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            return ip;
        }
        if let Some(ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            return ip.trim().to_string();
        }
    }
    peer.map(|p| p.ip().to_string()).unwrap_or_else(|| "unknown".into())
}

/// Rejects cross-site state-changing requests on custom routes: if an
/// `Origin` header is present it must match the `Host`.
pub fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let origin_host = origin
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    origin_host.eq_ignore_ascii_case(host)
}

/// Cache policy for static files: content-hashed `/pkg` assets are immutable
/// (when `LEPTOS_HASH_FILES` is on), versioned scripts and vendor libraries
/// are immutable per version query, the rest revalidates. Everything else
/// keeps its own headers.
pub async fn static_cache_headers(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = req.uri().path().to_string();
    let versioned = req.uri().query().is_some_and(|q| q.starts_with("v="));
    let mut res = next.run(req).await;
    if !res.status().is_success() || res.headers().contains_key(header::CACHE_CONTROL) {
        return res;
    }
    let hashed = std::env::var("LEPTOS_HASH_FILES").is_ok_and(|v| v == "true");
    let pkg = path.starts_with("/pkg/");
    let script = path.starts_with("/js/") || path.starts_with("/vendor/");
    let value = if (pkg && hashed) || (script && versioned) {
        Some("public, max-age=31536000, immutable")
    } else if pkg || script {
        Some("no-cache")
    } else if path == "/favicon.svg" {
        Some("public, max-age=86400")
    } else {
        None
    };
    if let Some(v) = value {
        let headers = res.headers_mut();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(v));
        // Release builds ship .br/.gz copies; shared caches must key on the encoding.
        headers.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    }
    res
}
