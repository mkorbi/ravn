//! Phase 5.3: Bearer-token + IP-allowlist auth for the **HTTP** transport.
//!
//! stdio needs no auth — it's a local subprocess the operator launched. For
//! HTTP, [`require_auth`] is an axum middleware layered in front of `/mcp`:
//! it checks the peer IP against the allowlist, then the `Authorization:
//! Bearer …` header. Either check is skipped when its config is empty, so the
//! default (loopback bind, no token) keeps working unchanged.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::config::HttpConfig;

#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    /// Required `Bearer` token. Empty disables the check.
    pub bearer_token: String,
    /// Permitted peer IPs. Empty disables the check (any IP allowed).
    pub ip_allowlist: Vec<IpAddr>,
}

impl AuthConfig {
    /// Build from parsed [`HttpConfig`]; unparseable allowlist entries are
    /// logged and dropped rather than silently widening access.
    pub fn from_http(http: &HttpConfig) -> Self {
        let ip_allowlist = http
            .ip_allowlist
            .iter()
            .filter_map(|s| match s.parse::<IpAddr>() {
                Ok(ip) => Some(ip),
                Err(_) => {
                    tracing::warn!(entry = %s, "ignoring invalid ip_allowlist entry");
                    None
                }
            })
            .collect();
        Self {
            bearer_token: http.bearer_token.clone(),
            ip_allowlist,
        }
    }

    /// True if at least one check is active.
    pub fn is_enabled(&self) -> bool {
        !self.bearer_token.is_empty() || !self.ip_allowlist.is_empty()
    }
}

/// Middleware enforcing [`AuthConfig`]. IP allowlist first (cheap, no header
/// parsing), then the Bearer token (constant-time compared).
pub async fn require_auth(
    State(cfg): State<Arc<AuthConfig>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if !cfg.ip_allowlist.is_empty() && !cfg.ip_allowlist.contains(&peer.ip()) {
        tracing::warn!(peer = %peer.ip(), "rejected: ip not in allowlist");
        return (StatusCode::FORBIDDEN, "forbidden: ip not allowed\n").into_response();
    }

    if !cfg.bearer_token.is_empty() {
        let presented = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        let ok = presented
            .map(|t| ct_eq(t.as_bytes(), cfg.bearer_token.as_bytes()))
            .unwrap_or(false);
        if !ok {
            tracing::warn!(peer = %peer.ip(), "rejected: missing or invalid bearer token");
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
    }

    next.run(req).await
}

/// Length-checked constant-time byte comparison — no early-exit timing oracle
/// on the token contents.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_matches_and_rejects() {
        assert!(ct_eq(b"secret", b"secret"));
        assert!(!ct_eq(b"secret", b"secrer"));
        assert!(!ct_eq(b"secret", b"secre"));
        assert!(!ct_eq(b"", b"x"));
    }

    #[test]
    fn from_http_parses_and_filters_ips() {
        let http = HttpConfig {
            enabled: true,
            bind: "127.0.0.1:8787".into(),
            bearer_token: "tok".into(),
            ip_allowlist: vec!["10.0.0.1".into(), "not-an-ip".into(), "::1".into()],
        };
        let auth = AuthConfig::from_http(&http);
        assert!(auth.is_enabled());
        assert_eq!(auth.ip_allowlist.len(), 2); // bad entry dropped
        assert!(auth.ip_allowlist.contains(&"10.0.0.1".parse().unwrap()));
        assert!(auth.ip_allowlist.contains(&"::1".parse().unwrap()));
    }

    #[test]
    fn disabled_when_empty() {
        assert!(!AuthConfig::default().is_enabled());
    }
}
