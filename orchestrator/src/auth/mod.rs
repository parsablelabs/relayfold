//! Installation-wide OAuth credentials. Workers never receive refresh tokens.
pub mod cli;
pub mod codex;
pub mod store;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use store::FileCredentialStore;

#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthCredential {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_at: i64,
    pub account_id: String,
}

#[derive(Serialize)]
pub struct AccessCredential {
    pub access_token: String,
    pub expires_at: i64,
}

#[async_trait]
pub trait OAuthProvider: Send + Sync {
    async fn refresh(&self, credential: &OAuthCredential) -> anyhow::Result<OAuthCredential>;
}

#[derive(Clone)]
pub struct AuthService {
    store: FileCredentialStore,
    provider: Arc<dyn OAuthProvider>,
}

impl AuthService {
    pub fn new(store: FileCredentialStore, provider: Arc<dyn OAuthProvider>) -> Self {
        Self { store, provider }
    }

    pub async fn access(&self) -> anyhow::Result<AccessCredential> {
        // This file lock also coordinates with the standalone login/logout CLI.
        let locked = self.store.lock().await?;
        let mut credential = locked.read()?.ok_or_else(|| {
            anyhow::anyhow!("Codex is not connected; run orchestrator login openai-codex")
        })?;
        if credential.expires_at <= chrono::Utc::now().timestamp_millis() + 60_000 {
            credential = self.provider.refresh(&credential).await?;
            locked.write(Some(&credential))?;
        }
        Ok(AccessCredential {
            access_token: credential.access_token,
            expires_at: credential.expires_at,
        })
    }
}

pub fn router(service: AuthService) -> axum::Router {
    use axum::{
        Json,
        extract::{Path, State},
        http::{HeaderMap, StatusCode, header},
        routing::post,
    };
    async fn access(
        State(service): State<AuthService>,
        Path(provider): Path<String>,
    ) -> Result<(HeaderMap, Json<AccessCredential>), (StatusCode, Json<serde_json::Value>)> {
        if provider != "openai-codex" {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Unsupported OAuth model provider"})),
            ));
        }
        let credential = service.access().await.map_err(|error| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": error.to_string()})),
            )
        })?;
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
        Ok((headers, Json(credential)))
    }
    // Intentionally unauthenticated on the trusted worker network (ADR 0003).
    axum::Router::new()
        .route("/workers/auth/{provider}", post(access))
        .with_state(service)
}

#[cfg(test)]
mod tests;
