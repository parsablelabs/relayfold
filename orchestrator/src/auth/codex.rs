use super::{OAuthCredential, OAuthProvider};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};

// Matches the Codex client/route used by the installed Pi Codex provider.
// End-to-end provider compatibility is verified manually (ADR 0003).
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";

pub struct LoginAttempt {
    verifier: String,
    pub state: String,
    pub url: String,
}

impl LoginAttempt {
    pub fn new() -> Self {
        let mut random = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut random);
        let verifier = URL_SAFE_NO_PAD.encode(random);
        rand::thread_rng().fill_bytes(&mut random);
        let state = URL_SAFE_NO_PAD.encode(random);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = reqwest::Url::parse("https://auth.openai.com/oauth/authorize").unwrap();
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            // Required Codex client scopes, matching Pi; exact minimum is live-verified.
            ("scope", "openid profile email offline_access"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "relayfold"),
        ]);
        Self {
            verifier,
            state,
            url: url.to_string(),
        }
    }

    pub fn code(&self, input: &str) -> anyhow::Result<String> {
        let url = reqwest::Url::parse(input.trim()).map_err(|_| {
            anyhow::anyhow!("Paste the full callback URL, including code and state")
        })?;
        anyhow::ensure!(
            url.scheme() == "http"
                && url.host_str() == Some("localhost")
                && url.port_or_known_default() == Some(1455)
                && url.path() == "/auth/callback",
            "Unexpected OAuth callback URL"
        );
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        anyhow::ensure!(
            params.get("state") == Some(&self.state),
            "OAuth state mismatch"
        );
        anyhow::ensure!(
            !params.contains_key("error"),
            "Codex authorization was declined or failed"
        );
        params
            .get("code")
            .filter(|code| !code.is_empty())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("OAuth callback has no authorization code"))
    }
}

pub struct CodexProvider {
    client: reqwest::Client,
}

impl CodexProvider {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }

    pub async fn exchange(
        &self,
        attempt: &LoginAttempt,
        code: &str,
    ) -> anyhow::Result<OAuthCredential> {
        self.token(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", &attempt.verifier),
            ("redirect_uri", REDIRECT_URI),
        ])
        .await
    }

    async fn token(&self, form: &[(&str, &str)]) -> anyhow::Result<OAuthCredential> {
        let response = self
            .client
            .post(TOKEN_URL)
            .form(form)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Unable to reach Codex token endpoint"))?;
        anyhow::ensure!(
            response.status().is_success(),
            "Codex token exchange failed (HTTP {}); reconnect with orchestrator login openai-codex if authorization is invalid",
            response.status().as_u16()
        );
        let token: TokenResponse = response
            .json()
            .await
            .map_err(|_| anyhow::anyhow!("Invalid Codex token response"))?;
        token.into_credential(chrono::Utc::now().timestamp_millis())
    }
}

#[async_trait]
impl OAuthProvider for CodexProvider {
    async fn refresh(&self, credential: &OAuthCredential) -> anyhow::Result<OAuthCredential> {
        self.token(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", &credential.refresh_token),
        ])
        .await
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    token_type: String,
}

impl TokenResponse {
    fn into_credential(self, now: i64) -> anyhow::Result<OAuthCredential> {
        anyhow::ensure!(
            !self.access_token.is_empty()
                && !self.refresh_token.is_empty()
                && self.expires_in > 0
                && self.token_type.eq_ignore_ascii_case("bearer"),
            "Incomplete Codex token response"
        );
        // Decode provider-issued metadata, not identity authentication. Token was
        // obtained over TLS from the provider; Pi extracts the same account claim.
        let payload = self
            .access_token
            .split('.')
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Codex token has no account metadata"))?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| anyhow::anyhow!("Invalid Codex account metadata"))?;
        let claims: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid Codex account metadata"))?;
        let account_id = claims["https://api.openai.com/auth"]["chatgpt_account_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Codex token has no account ID"))?
            .to_owned();
        let expires_at = self
            .expires_in
            .checked_mul(1000)
            .and_then(|ms| now.checked_add(ms))
            .ok_or_else(|| anyhow::anyhow!("Invalid Codex token expiry"))?;
        Ok(OAuthCredential {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            token_type: self.token_type,
            expires_at,
            account_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_requires_matching_state_and_redirect() {
        let attempt = LoginAttempt::new();
        assert!(
            attempt
                .code("http://localhost:1455/auth/callback?code=code&state=wrong")
                .is_err()
        );
        assert!(attempt.code("raw-code").is_err());
        assert!(
            attempt
                .code(&format!(
                    "https://other.test/auth/callback?code=code&state={}",
                    attempt.state
                ))
                .is_err()
        );
        assert_eq!(
            attempt
                .code(&format!("{REDIRECT_URI}?code=code&state={}", attempt.state))
                .unwrap(),
            "code"
        );
        let url = reqwest::Url::parse(&attempt.url).unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            params["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(attempt.verifier.as_bytes()))
        );
    }
    #[test]
    fn token_mapping_retains_account_and_absolute_expiry() {
        let payload = URL_SAFE_NO_PAD
            .encode(br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"account"}}"#);
        let response = TokenResponse {
            access_token: format!("header.{payload}.signature"),
            refresh_token: "refresh".into(),
            token_type: "Bearer".into(),
            expires_in: 3600,
        };
        let credential = response.into_credential(1000).unwrap();
        assert_eq!(credential.expires_at, 3_601_000);
        assert_eq!(credential.account_id, "account");
    }
}
