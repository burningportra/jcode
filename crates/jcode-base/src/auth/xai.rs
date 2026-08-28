//! Native xAI Grok OAuth (device-code) login, token storage, and refresh.
//!
//! This is the `xai-oauth` provider: a SuperGrok / X Premium+ subscription
//! login that is deliberately separate from both the API-key `xai` provider
//! (which consumes `XAI_API_KEY`) and the Jcode-managed `grok-build` provider.
//!
//! The flow is RFC 8628 OAuth 2.0 Device Authorization Grant against xAI's
//! public Grok-CLI client. All constants below were verified against the live
//! OIDC discovery document at `https://auth.x.ai/.well-known/openid-configuration`
//! and by probing the device/token endpoints directly:
//!
//! - The client is a *public* client (`token_endpoint_auth_methods_supported`
//!   includes `"none"`), so no client secret is required or sent.
//! - `POST /oauth2/device/code` with `client_id` + `scope` returns
//!   `{device_code, user_code, verification_uri, verification_uri_complete,
//!    expires_in, interval}`.
//! - Polling `POST /oauth2/token` with the device-code grant + `client_id`
//!   returns `authorization_pending` until the user approves, then the tokens.
//!
//! Runtime inference reuses the OpenAI-compatible transport against
//! `https://api.x.ai/v1`; this module only owns auth (login/storage/refresh)
//! and hands a bearer to the runtime via [`resolve_bearer`].

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

/// OIDC issuer for xAI accounts.
pub const XAI_OAUTH_ISSUER: &str = "https://auth.x.ai";
/// Device authorization endpoint (RFC 8628).
const XAI_DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
/// Token endpoint (device-code grant + refresh).
const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
/// Token revocation endpoint (best-effort on logout).
const XAI_REVOKE_URL: &str = "https://auth.x.ai/oauth2/revoke";

/// Public Grok-CLI OAuth client id. This is a public client with no secret
/// (verified: the token endpoint accepts the device-code grant with only the
/// client id). Overridable via `JCODE_XAI_OAUTH_CLIENT_ID` for future-proofing.
const XAI_OAUTH_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_OAUTH_CLIENT_ID_ENV: &str = "JCODE_XAI_OAUTH_CLIENT_ID";

/// Scopes: `offline_access` yields a refresh token; `api:access` and
/// `grok-cli:access` authorize inference. Overridable via env.
const XAI_OAUTH_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const XAI_OAUTH_SCOPE_ENV: &str = "JCODE_XAI_OAUTH_SCOPE";

/// Device-code grant type (RFC 8628).
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// xAI OAuth access tokens are short-lived (~6h in current SuperGrok flows).
/// Refresh up to one hour early so occasional/cron workloads keep a warm token
/// instead of hitting brief expiry gaps. This is intentionally far larger than
/// the 60s margin used for Google tokens.
const XAI_REFRESH_SKEW_MS: i64 = 3_600_000;

/// Provider id used for refresh-state bookkeeping and single-flight keys.
pub const XAI_OAUTH_PROVIDER_ID: &str = "xai-oauth";

/// Hint shown when a refresh is permanently rejected.
const RELOGIN_HINT: &str = "Run `jcode login --provider xai-oauth` to sign in again.";

fn client_id() -> String {
    std::env::var(XAI_OAUTH_CLIENT_ID_ENV).unwrap_or_else(|_| XAI_OAUTH_CLIENT_ID.to_string())
}

fn scope() -> String {
    std::env::var(XAI_OAUTH_SCOPE_ENV).unwrap_or_else(|_| XAI_OAUTH_SCOPE.to_string())
}

/// Persisted xAI OAuth credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XaiTokens {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Absolute expiry in unix milliseconds.
    pub expires_at: i64,
    #[serde(default = "default_token_type")]
    pub token_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// OIDC id_token (ES256 JWT) if returned; used only for display, never sent
    /// to the inference API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    /// When these tokens were obtained (unix ms).
    #[serde(default)]
    pub obtained_at: i64,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

impl XaiTokens {
    /// True when the access token is expired or within the refresh skew window.
    pub fn is_expired(&self) -> bool {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.expires_at <= now_ms + XAI_REFRESH_SKEW_MS
    }
}

/// Raw token-endpoint response (device grant and refresh share this shape).
#[derive(Debug, Deserialize)]
struct XaiTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

impl XaiTokenResponse {
    /// Convert a token response into stored tokens, preserving `prior_refresh`
    /// when the response omits a rotated refresh token.
    fn into_tokens(self, prior_refresh: Option<String>) -> XaiTokens {
        let now_ms = chrono::Utc::now().timestamp_millis();
        // Default to ~6h if the server omits expires_in.
        let expires_in = self.expires_in.unwrap_or(6 * 60 * 60);
        XaiTokens {
            access_token: self.access_token,
            refresh_token: self.refresh_token.or(prior_refresh),
            expires_at: now_ms + expires_in * 1000,
            token_type: self.token_type.unwrap_or_else(default_token_type),
            scope: self.scope,
            id_token: self.id_token,
            obtained_at: now_ms,
        }
    }
}

/// Device-authorization response (RFC 8628).
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: i64,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

/// Path to the native xAI OAuth token store.
pub fn tokens_path() -> Result<std::path::PathBuf> {
    Ok(crate::storage::jcode_dir()?.join("xai_oauth.json"))
}

/// True when native xAI OAuth tokens are on disk.
pub fn has_cached_auth() -> bool {
    load_tokens().is_ok()
}

/// Load stored tokens. v1 discovery order is the native file only.
pub fn load_tokens() -> Result<XaiTokens> {
    let path = tokens_path()?;
    if !path.exists() {
        anyhow::bail!("No xAI OAuth tokens found. Run `jcode login --provider xai-oauth`.");
    }
    crate::storage::harden_secret_file_permissions(&path);
    crate::storage::read_json(&path)
        .with_context(|| format!("Failed to read {}", path.display()))
}

/// Persist tokens with hardened (0600) permissions.
pub fn save_tokens(tokens: &XaiTokens) -> Result<()> {
    let path = tokens_path()?;
    crate::storage::write_json_secret(&path, tokens)?;
    super::AuthStatus::invalidate_cache();
    Ok(())
}

/// Remove stored tokens (used by logout). Best-effort revocation is handled by
/// [`logout`].
pub fn clear_tokens() -> Result<()> {
    let path = tokens_path()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to remove {}", path.display()))?;
    }
    super::AuthStatus::invalidate_cache();
    Ok(())
}

/// Return a valid bearer token for the runtime, refreshing if necessary.
pub async fn resolve_bearer() -> Result<String> {
    let tokens = load_or_refresh_tokens().await?;
    Ok(tokens.access_token)
}

/// Load tokens, refreshing when expired (or within the skew window).
pub async fn load_or_refresh_tokens() -> Result<XaiTokens> {
    let tokens = load_tokens()?;
    if tokens.is_expired() {
        refresh_tokens(&tokens).await
    } else {
        Ok(tokens)
    }
}

/// Refresh the access token, serialized per-provider so concurrent callers do
/// not race the token endpoint or the stored file.
pub async fn refresh_tokens(tokens: &XaiTokens) -> Result<XaiTokens> {
    crate::auth::refresh_coordinator::single_flight(
        XAI_OAUTH_PROVIDER_ID.to_string(),
        || load_tokens().ok(),
        |stored: &XaiTokens| !stored.is_expired(),
        {
            let observed = tokens.clone();
            move |stored: Option<XaiTokens>| async move {
                let source = stored.unwrap_or(observed);
                refresh_tokens_uncoordinated(&source).await
            }
        },
    )
    .await
}

async fn refresh_tokens_uncoordinated(tokens: &XaiTokens) -> Result<XaiTokens> {
    let Some(refresh_token) = tokens.refresh_token.clone().filter(|t| !t.trim().is_empty())
    else {
        anyhow::bail!(
            "xAI OAuth session has no refresh token; sign in again. {RELOGIN_HINT}"
        );
    };

    // Skip a doomed round-trip if this exact token was already rejected.
    crate::auth::refresh_state::ensure_refresh_allowed(
        XAI_OAUTH_PROVIDER_ID,
        &refresh_token,
        RELOGIN_HINT,
    )?;

    let result: Result<XaiTokens> = async {
        let client = crate::provider::shared_http_client();
        let resp = client
            .post(XAI_TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", &client_id()),
                ("refresh_token", &refresh_token),
            ])
            .send()
            .await
            .context("Failed to refresh xAI OAuth token")?;

        if !resp.status().is_success() {
            let body = crate::util::http_error_body(resp, "HTTP error").await;
            anyhow::bail!("xAI token refresh failed: {}", body.trim());
        }

        let token_resp: XaiTokenResponse = resp
            .json()
            .await
            .context("Failed to parse xAI refresh response")?;
        let refreshed = token_resp.into_tokens(Some(refresh_token.clone()));
        save_tokens(&refreshed)?;
        Ok(refreshed)
    }
    .await;

    // Route permanent rejections (invalid_grant, revoked) into the terminal
    // state so background sweeps stop retrying; transient failures stay
    // retryable.
    crate::auth::refresh_state::record_refresh_outcome(
        XAI_OAUTH_PROVIDER_ID,
        &refresh_token,
        &result,
    );

    result
}

/// Options controlling the interactive device-code login.
#[derive(Debug, Clone, Default)]
pub struct LoginOptions {
    /// When true, never attempt to open a browser (headless / SSH).
    pub no_browser: bool,
    /// When true, print the verification URL + code and return without polling
    /// (scriptable flow). The caller completes the login out of band.
    pub print_only: bool,
}

/// Run the device-code login flow and persist the resulting tokens.
pub async fn login(options: LoginOptions) -> Result<XaiTokens> {
    let device = request_device_code().await?;

    let verification_url = device
        .verification_uri_complete
        .clone()
        .unwrap_or_else(|| device.verification_uri.clone());

    eprintln!("\nSign in to xAI Grok (SuperGrok / X Premium+).\n");
    eprintln!("1. Open this URL in your browser:\n\n   {}\n", device.verification_uri);
    eprintln!("2. Enter this code if prompted:  {}\n", device.user_code);
    if let Some(qr) = crate::login_qr::indented_section(
        &verification_url,
        "Or scan this QR on another device:",
        "    ",
    ) {
        eprintln!("{qr}\n");
    }

    if !crate::auth::browser_suppressed(options.no_browser) {
        let _ = open::that(&verification_url);
    }

    if options.print_only {
        eprintln!(
            "Approve the login in your browser, then re-run without --print-auth-url to complete."
        );
        anyhow::bail!(
            "Printed the xAI verification URL. Device-code polling was skipped (--print-auth-url)."
        );
    }

    eprintln!("Waiting for approval (this can take up to {}s)...", device.expires_in);
    let tokens = poll_for_token(&device).await?;
    save_tokens(&tokens)?;
    let _ = crate::auth::refresh_state::record_success(XAI_OAUTH_PROVIDER_ID);
    eprintln!("\nxAI Grok OAuth login complete.");
    Ok(tokens)
}

/// Start the device-code flow (public wrapper used by the TUI). Returns the
/// device authorization the caller shows to the user, then completes with
/// [`complete_device_login`].
pub async fn initiate_device_login() -> Result<DeviceCodeResponse> {
    request_device_code().await
}

/// Poll to completion for a device authorization previously obtained via
/// [`initiate_device_login`], persisting tokens on success.
pub async fn complete_device_login(device: &DeviceCodeResponse) -> Result<XaiTokens> {
    let tokens = poll_for_token(device).await?;
    save_tokens(&tokens)?;
    let _ = crate::auth::refresh_state::record_success(XAI_OAUTH_PROVIDER_ID);
    Ok(tokens)
}

async fn request_device_code() -> Result<DeviceCodeResponse> {
    let client = crate::provider::shared_http_client();
    let resp = client
        .post(XAI_DEVICE_CODE_URL)
        .form(&[("client_id", client_id()), ("scope", scope())])
        .send()
        .await
        .context("Failed to request xAI device code")?;

    if !resp.status().is_success() {
        let body = crate::util::http_error_body(resp, "HTTP error").await;
        anyhow::bail!("xAI device authorization failed: {}", body.trim());
    }

    resp.json()
        .await
        .context("Failed to parse xAI device code response")
}

/// Terminal error the token endpoint can return while polling.
#[derive(Debug, Deserialize)]
struct TokenErrorBody {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

async fn poll_for_token(device: &DeviceCodeResponse) -> Result<XaiTokens> {
    let client = crate::provider::shared_http_client();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(device.expires_in.max(0) as u64);
    let mut interval = device.interval.max(1);

    loop {
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "xAI login timed out before approval. Re-run `jcode login --provider xai-oauth`."
            );
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;

        let resp = client
            .post(XAI_TOKEN_URL)
            .form(&[
                ("grant_type", DEVICE_CODE_GRANT),
                ("client_id", &client_id()),
                ("device_code", &device.device_code),
            ])
            .send()
            .await
            .context("Failed to poll xAI token endpoint")?;

        if resp.status().is_success() {
            let token_resp: XaiTokenResponse = resp
                .json()
                .await
                .context("Failed to parse xAI token response")?;
            return Ok(token_resp.into_tokens(None));
        }

        // Non-success: parse the OAuth error code to decide next action.
        let body = resp.text().await.unwrap_or_default();
        match classify_poll_error(&body) {
            PollAction::KeepPolling => continue,
            PollAction::SlowDown => {
                interval += 5;
                continue;
            }
            PollAction::Denied => {
                anyhow::bail!("xAI login was denied. Re-run `jcode login --provider xai-oauth`.")
            }
            PollAction::Expired => anyhow::bail!(
                "xAI device code expired before approval. Re-run `jcode login --provider xai-oauth`."
            ),
            PollAction::Fatal(message) => anyhow::bail!("xAI login failed: {message}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PollAction {
    KeepPolling,
    SlowDown,
    Denied,
    Expired,
    Fatal(String),
}

/// Map an RFC 8628 token-endpoint error body to a poll action.
fn classify_poll_error(body: &str) -> PollAction {
    let (code, description) = match serde_json::from_str::<TokenErrorBody>(body) {
        Ok(parsed) => (parsed.error, parsed.error_description),
        Err(_) => (body.trim().to_string(), None),
    };
    match code.as_str() {
        "authorization_pending" => PollAction::KeepPolling,
        "slow_down" => PollAction::SlowDown,
        "access_denied" => PollAction::Denied,
        "expired_token" => PollAction::Expired,
        other => PollAction::Fatal(description.unwrap_or_else(|| other.to_string())),
    }
}

/// Best-effort logout: revoke the token upstream, then clear the local store.
pub async fn logout() -> Result<()> {
    if let Ok(tokens) = load_tokens() {
        let token = tokens
            .refresh_token
            .clone()
            .unwrap_or(tokens.access_token.clone());
        let client = crate::provider::shared_http_client();
        // Revocation is best-effort; ignore failures.
        let _ = client
            .post(XAI_REVOKE_URL)
            .form(&[("client_id", client_id()), ("token", token)])
            .send()
            .await;
    }
    clear_tokens()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_with_expiry(expires_at: i64, refresh: Option<&str>) -> XaiTokens {
        XaiTokens {
            access_token: "access".into(),
            refresh_token: refresh.map(|r| r.to_string()),
            expires_at,
            token_type: "Bearer".into(),
            scope: None,
            id_token: None,
            obtained_at: 0,
        }
    }

    #[test]
    fn is_expired_uses_one_hour_skew() {
        let now = chrono::Utc::now().timestamp_millis();
        // 30 min in the future: within the 1h skew -> considered expired.
        assert!(tokens_with_expiry(now + 30 * 60_000, None).is_expired());
        // 2h in the future: outside the skew -> fresh.
        assert!(!tokens_with_expiry(now + 2 * 60 * 60_000, None).is_expired());
    }

    #[test]
    fn into_tokens_preserves_prior_refresh_when_omitted() {
        let resp = XaiTokenResponse {
            access_token: "new-access".into(),
            refresh_token: None,
            expires_in: Some(21_600),
            token_type: Some("Bearer".into()),
            scope: Some("api:access".into()),
            id_token: None,
        };
        let tokens = resp.into_tokens(Some("prior-refresh".into()));
        assert_eq!(tokens.access_token, "new-access");
        assert_eq!(tokens.refresh_token.as_deref(), Some("prior-refresh"));
        assert!(tokens.expires_at > chrono::Utc::now().timestamp_millis());
    }

    #[test]
    fn into_tokens_prefers_rotated_refresh() {
        let resp = XaiTokenResponse {
            access_token: "a".into(),
            refresh_token: Some("rotated".into()),
            expires_in: None,
            token_type: None,
            scope: None,
            id_token: None,
        };
        let tokens = resp.into_tokens(Some("prior".into()));
        assert_eq!(tokens.refresh_token.as_deref(), Some("rotated"));
        // Default ~6h expiry when server omits expires_in.
        assert!(tokens.expires_at > chrono::Utc::now().timestamp_millis() + 5 * 60 * 60_000);
    }

    #[test]
    fn classify_poll_error_maps_rfc8628_codes() {
        assert_eq!(
            classify_poll_error(r#"{"error":"authorization_pending"}"#),
            PollAction::KeepPolling
        );
        assert_eq!(
            classify_poll_error(r#"{"error":"slow_down"}"#),
            PollAction::SlowDown
        );
        assert_eq!(
            classify_poll_error(r#"{"error":"access_denied"}"#),
            PollAction::Denied
        );
        assert_eq!(
            classify_poll_error(r#"{"error":"expired_token"}"#),
            PollAction::Expired
        );
        assert!(matches!(
            classify_poll_error(r#"{"error":"invalid_client","error_description":"bad"}"#),
            PollAction::Fatal(_)
        ));
    }

    #[test]
    fn classify_poll_error_handles_non_json_body() {
        assert!(matches!(
            classify_poll_error("upstream 500"),
            PollAction::Fatal(_)
        ));
    }

    #[test]
    fn token_type_defaults_to_bearer_on_deserialize() {
        let tokens: XaiTokens = serde_json::from_str(
            r#"{"access_token":"a","expires_at":123}"#,
        )
        .unwrap();
        assert_eq!(tokens.token_type, "Bearer");
        assert!(tokens.refresh_token.is_none());
    }
}
