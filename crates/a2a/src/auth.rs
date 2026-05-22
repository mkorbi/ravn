//! OAuth2/OIDC bearer-token validation for the A2A server (Phase 5.5).
//!
//! We hand-roll the A2A protocol but **not** the crypto: tokens are validated
//! with `jsonwebtoken` against the IdP's JWKS (signature + issuer + audience +
//! expiry), plus a scope check. Auth is opt-in via the `[auth]` config block;
//! when absent the server runs unauthenticated (dev only).

use std::collections::HashSet;

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::config::AuthConfig;

#[derive(Debug, Deserialize, Default)]
struct Claims {
    /// OAuth2 space-delimited scopes.
    #[serde(default)]
    scope: Option<String>,
    /// Some IdPs use a `scp` array instead.
    #[serde(default)]
    scp: Option<Vec<String>>,
}

/// Collect granted scopes from the `scope` (space-delimited) and/or `scp`
/// (array) claims.
fn parse_scopes(scope: Option<&str>, scp: Option<&[String]>) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Some(s) = scope {
        out.extend(s.split_whitespace().map(String::from));
    }
    if let Some(arr) = scp {
        out.extend(arr.iter().cloned());
    }
    out
}

fn has_required_scopes(granted: &HashSet<String>, required: &[String]) -> bool {
    required.iter().all(|r| granted.contains(r))
}

/// Validates incoming bearer JWTs against a configured JWKS.
pub struct JwtValidator {
    config: AuthConfig,
    http: reqwest::Client,
    jwks: RwLock<JwkSet>,
}

impl JwtValidator {
    /// Fetch the JWKS up front (fails fast on a misconfigured `[auth]`).
    pub async fn new(config: AuthConfig) -> anyhow::Result<Self> {
        let http = reqwest::Client::new();
        let jwks = fetch_jwks(&http, &config.jwks_url).await?;
        Ok(Self {
            config,
            http,
            jwks: RwLock::new(jwks),
        })
    }

    /// Validate a bearer token. `Ok(())` ⇒ accepted.
    pub async fn validate(&self, token: &str) -> Result<(), String> {
        let header = decode_header(token).map_err(|e| format!("bad token header: {e}"))?;
        let kid = header.kid.ok_or_else(|| "token missing kid".to_string())?;

        let jwk = match self.jwks.read().await.find(&kid).cloned() {
            Some(j) => j,
            None => {
                // Possible key rotation — refetch once.
                let fresh = fetch_jwks(&self.http, &self.config.jwks_url)
                    .await
                    .map_err(|e| format!("jwks refetch: {e}"))?;
                let found = fresh.find(&kid).cloned();
                *self.jwks.write().await = fresh;
                found.ok_or_else(|| format!("no signing key for kid {kid}"))?
            }
        };

        let key = DecodingKey::from_jwk(&jwk).map_err(|e| format!("jwk: {e}"))?;
        let mut v = Validation::new(header.alg);
        v.set_issuer(&[self.config.issuer.as_str()]);
        v.set_audience(&[self.config.audience.as_str()]);
        let data =
            decode::<Claims>(token, &key, &v).map_err(|e| format!("token rejected: {e}"))?;

        if !self.config.required_scopes.is_empty() {
            let granted = parse_scopes(data.claims.scope.as_deref(), data.claims.scp.as_deref());
            if !has_required_scopes(&granted, &self.config.required_scopes) {
                return Err("insufficient scope".to_string());
            }
        }
        Ok(())
    }
}

async fn fetch_jwks(http: &reqwest::Client, url: &str) -> anyhow::Result<JwkSet> {
    let set = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json::<JwkSet>()
        .await?;
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scope_string_and_scp_array() {
        let s = parse_scopes(Some("a2a.invoke read"), None);
        assert!(s.contains("a2a.invoke") && s.contains("read"));
        let s = parse_scopes(None, Some(&["x".to_string(), "y".to_string()]));
        assert!(s.contains("x") && s.contains("y"));
    }

    #[test]
    fn required_scopes_subset_check() {
        let granted = parse_scopes(Some("a b c"), None);
        assert!(has_required_scopes(&granted, &["a".into(), "b".into()]));
        assert!(!has_required_scopes(&granted, &["a".into(), "z".into()]));
        assert!(has_required_scopes(&granted, &[])); // none required ⇒ ok
    }
}
