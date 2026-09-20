//! DeskModal institutional JWT verification engine.
//!
//! Provides cryptographic validation of DeskModal-issued authentication tokens
//! (HS256), enabling zero-touch Single Sign-On (SSO) pass-through from DeskModal
//! desktop host directly into CelNet trading services.
//!
//! Conforms to:
//! - RFC 7519 (JSON Web Token)
//! - RFC 7515 (JSON Web Signature)
//! - DeskModal `core-server-api` JWT schema (`auth/jwt.rs`)
//! - DeskModal institutional security doctrine (tamper-evident, zero mocks)

use std::env;
use std::sync::OnceLock;

use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::config::identity::{Role, default_trader_bundle};
use crate::services::sessions::AuthenticatedUser;

type HmacSha256 = Hmac<Sha256>;

/// Default dev JWT secret matching `core-server-api/dev.config.toml`.
pub const DEFAULT_DEV_JWT_SECRET: &str =
    "dev_secret_key_must_be_at_least_32_characters_long_1234567890";

/// Org membership claim matching DeskModal `core-server-api::auth::jwt::OrgMembership`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct OrgMembership {
    /// Organization identifier.
    pub org_id: String,
    /// Role within the organization (`admin`, `member`, `viewer`).
    pub role: String,
}

/// JWT Claims matching DeskModal `core-server-api::auth::jwt::Claims`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct DeskModalClaims {
    /// Subject -- the user id in DeskModal.
    pub sub: String,
    /// User email address.
    pub email: String,
    /// User role (`admin`, `trader`, etc.).
    pub role: String,
    /// Expiration time as UTC Unix timestamp (seconds).
    pub exp: i64,
    /// Issued-at time as UTC Unix timestamp (seconds).
    pub iat: i64,
    /// Multi-tenant org memberships.
    #[serde(default)]
    pub org_memberships: Vec<OrgMembership>,
}

/// JWT Header.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JwtHeader {
    alg: String,
    #[serde(default)]
    typ: Option<String>,
}

/// Active JWT signing secret provider.
/// Resolves from `CELNET_JWT_KEY` or `DESKMODAL_JWT_SECRET` environment variables,
/// falling back to the canonical `DEFAULT_DEV_JWT_SECRET`.
pub fn get_jwt_secret() -> &'static str {
    static SECRET: OnceLock<String> = OnceLock::new();
    SECRET.get_or_init(|| {
        env::var("CELNET_JWT_KEY")
            .or_else(|_| env::var("DESKMODAL_JWT_SECRET"))
            .unwrap_or_else(|_| DEFAULT_DEV_JWT_SECRET.to_string())
    })
}

/// Decode Base64URL bytes, handling both padded and unpadded variants.
fn decode_base64_url(input: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(input.trim())
        .or_else(|_| URL_SAFE.decode(input.trim()))
        .map_err(|e| format!("invalid base64url: {e}"))
}

/// Cryptographically verify a DeskModal JWT token and extract verified claims.
///
/// Validates:
/// 1. Three-part dot-delimited structure (`header.payload.signature`)
/// 2. Header `alg == "HS256"`
/// 3. HMAC-SHA256 signature using `secret` (constant-time verification)
/// 4. Expiration timestamp (`exp > now_seconds`)
pub fn verify_deskmodal_jwt(
    token: &str,
    secret: &str,
    now_seconds: i64,
) -> Result<DeskModalClaims, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("JWT must have exactly 3 parts separated by dots".to_string());
    }

    let header_b64 = parts[0];
    let payload_b64 = parts[1];
    let signature_b64 = parts[2];

    // 1. Decode & validate header
    let header_bytes = decode_base64_url(header_b64)?;
    let header: JwtHeader = serde_json::from_slice(&header_bytes)
        .map_err(|e| format!("failed to parse JWT header: {e}"))?;

    if header.alg != "HS256" {
        return Err(format!("unsupported JWT algorithm: {}", header.alg));
    }

    // 2. Verify cryptographic signature with constant-time equality
    let signing_input = format!("{header_b64}.{payload_b64}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("HMAC key initialization error: {e}"))?;
    mac.update(signing_input.as_bytes());

    let signature_bytes = decode_base64_url(signature_b64)?;
    mac.verify_slice(&signature_bytes)
        .map_err(|_| "JWT signature mismatch: cryptographic verification failed".to_string())?;

    // 3. Decode & parse payload claims
    let payload_bytes = decode_base64_url(payload_b64)?;
    let claims: DeskModalClaims = serde_json::from_slice(&payload_bytes)
        .map_err(|e| format!("failed to parse JWT claims: {e}"))?;

    // 4. Validate expiration
    if claims.exp <= now_seconds {
        return Err(format!(
            "JWT token has expired (exp: {}, now: {})",
            claims.exp, now_seconds
        ));
    }

    Ok(claims)
}

/// Mint a cryptographically signed DeskModal JWT using HS256.
pub fn create_deskmodal_jwt(claims: &DeskModalClaims, secret: &str) -> Result<String, String> {
    let header = serde_json::json!({
        "typ": "JWT",
        "alg": "HS256"
    });
    let header_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).map_err(|e| e.to_string())?);
    let payload_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).map_err(|e| e.to_string())?);
    let signing_input = format!("{header_b64}.{payload_b64}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("HMAC key initialization error: {e}"))?;
    mac.update(signing_input.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = URL_SAFE_NO_PAD.encode(sig);
    Ok(format!("{signing_input}.{sig_b64}"))
}

/// Convert verified DeskModal claims into a CelNet `AuthenticatedUser`.
pub fn claims_to_authenticated_user(claims: &DeskModalClaims) -> AuthenticatedUser {
    let role = if claims.role.eq_ignore_ascii_case("admin") {
        Role::Admin
    } else {
        Role::Trader
    };

    let role_caps = default_trader_bundle();

    AuthenticatedUser {
        user_id: claims.sub.clone(),
        email: claims.email.clone(),
        display_name: if !claims.email.is_empty() {
            claims.email.clone()
        } else {
            claims.sub.clone()
        },
        role,
        desk_ids: vec![],
        all_desks: true,
        role_caps,
        cap_grants: vec![],
        cap_denies: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_real_deskmodal_jwt() {
        // Real JWT token from /Users/adrian/deskmodal/dist/data/agent.auth/storage/jwt.json
        let real_token = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJkZXYtdXNlci0wMDEiLCJlbWFpbCI6ImRldkBkZXNrbW9kYWwuY29tIiwicm9sZSI6ImFkbWluIiwiZXhwIjoxNzg5ODczODk1LCJpYXQiOjE3ODk4NzI5OTUsIm9yZ19tZW1iZXJzaGlwcyI6W119.wMKprXyOxQgmlz5whtrXdfZWKCmtG1vkIelO1kMxgwo";

        let secret = DEFAULT_DEV_JWT_SECRET;
        let claims = verify_deskmodal_jwt(real_token, secret, 1789873000).expect("valid jwt");

        assert_eq!(claims.sub, "dev-user-001");
        assert_eq!(claims.email, "dev@deskmodal.com");
        assert_eq!(claims.role, "admin");
        assert_eq!(claims.exp, 1789873895);

        let user = claims_to_authenticated_user(&claims);
        assert_eq!(user.user_id, "dev-user-001");
        assert_eq!(user.email, "dev@deskmodal.com");
        assert_eq!(user.role, Role::Admin);
        assert!(user.all_desks);
    }

    #[test]
    fn test_reject_tampered_jwt() {
        let real_token = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJkZXYtdXNlci0wMDEiLCJlbWFpbCI6ImRldkBkZXNrbW9kYWwuY29tIiwicm9sZSI6ImFkbWluIiwiZXhwIjoxNzg5ODczODk1LCJpYXQiOjE3ODk4NzI5OTUsIm9yZ19tZW1iZXJzaGlwcyI6W119.wMKprXyOxQgmlz5whtrXdfZWKCmtG1vkIelO1kMxgwo";
        let tampered = format!("{real_token}tampered");
        let err = verify_deskmodal_jwt(&tampered, DEFAULT_DEV_JWT_SECRET, 1789873000);
        assert!(err.is_err());
    }

    #[test]
    fn test_reject_expired_jwt() {
        let real_token = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJkZXYtdXNlci0wMDEiLCJlbWFpbCI6ImRldkBkZXNrbW9kYWwuY29tIiwicm9sZSI6ImFkbWluIiwiZXhwIjoxNzg5ODczODk1LCJpYXQiOjE3ODk4NzI5OTUsIm9yZ19tZW1iZXJzaGlwcyI6W119.wMKprXyOxQgmlz5whtrXdfZWKCmtG1vkIelO1kMxgwo";
        let err = verify_deskmodal_jwt(real_token, DEFAULT_DEV_JWT_SECRET, 1789874000);
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("expired"));
    }
}
