use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

struct FakeProvider {
    refreshes: AtomicUsize,
    entered: Notify,
    fail: bool,
}
#[async_trait]
impl OAuthProvider for FakeProvider {
    async fn refresh(&self, credential: &OAuthCredential) -> anyhow::Result<OAuthCredential> {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        anyhow::ensure!(!self.fail, "Refresh failed");
        let mut refreshed = credential.clone();
        refreshed.access_token = "renewed".into();
        refreshed.refresh_token = "rotated".into();
        refreshed.expires_at = chrono::Utc::now().timestamp_millis() + 3_600_000;
        Ok(refreshed)
    }
}

fn credential(expires_at: i64) -> OAuthCredential {
    OAuthCredential {
        access_token: "old".into(),
        refresh_token: "secret".into(),
        token_type: "Bearer".into(),
        expires_at,
        account_id: "account".into(),
    }
}
struct Fixture {
    directory: std::path::PathBuf,
    store: FileCredentialStore,
    provider: Arc<FakeProvider>,
}
impl Fixture {
    fn new(fail: bool) -> Self {
        let directory =
            std::env::temp_dir().join(format!("relayfold-oauth-{}", rand::random::<u64>()));
        Self {
            store: FileCredentialStore::new(directory.clone()),
            directory,
            provider: Arc::new(FakeProvider {
                refreshes: AtomicUsize::new(0),
                entered: Notify::new(),
                fail,
            }),
        }
    }
    fn service(&self) -> AuthService {
        AuthService::new(self.store.clone(), self.provider.clone())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[tokio::test]
async fn concurrent_refresh_is_persisted_once_and_workers_never_get_refresh_token() {
    let f = Fixture::new(false);
    f.store
        .lock()
        .await
        .unwrap()
        .write(Some(&credential(0)))
        .unwrap();
    let service_a = f.service();
    let service_b = f.service();
    let (a, b) = tokio::join!(service_a.access(), service_b.access());
    for access in [a.unwrap(), b.unwrap()] {
        assert_eq!(access.access_token, "renewed");
        let json = serde_json::to_value(access).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 2);
        assert!(json.get("refresh_token").is_none());
    }
    assert_eq!(f.provider.refreshes.load(Ordering::SeqCst), 1);
    let persisted = f.store.lock().await.unwrap().read().unwrap().unwrap();
    assert_eq!(persisted.refresh_token, "rotated");
    assert_eq!(f.service().access().await.unwrap().access_token, "renewed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(f.directory.join("openai-codex.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn logout_waits_for_refresh_and_prevents_recreation() {
    let f = Fixture::new(false);
    f.store
        .lock()
        .await
        .unwrap()
        .write(Some(&credential(0)))
        .unwrap();
    let service = f.service();
    let refreshing = tokio::spawn(async move { service.access().await });
    f.provider.entered.notified().await;
    f.store.lock().await.unwrap().write(None).unwrap();
    refreshing.await.unwrap().unwrap();
    assert!(f.store.lock().await.unwrap().read().unwrap().is_none());
    assert!(f.service().access().await.is_err());
}

#[tokio::test]
async fn replacement_is_not_overwritten_by_old_refresh() {
    let f = Fixture::new(false);
    f.store
        .lock()
        .await
        .unwrap()
        .write(Some(&credential(0)))
        .unwrap();
    let service = f.service();
    let refreshing = tokio::spawn(async move { service.access().await });
    f.provider.entered.notified().await;
    let mut replacement = credential(chrono::Utc::now().timestamp_millis() + 3_600_000);
    replacement.access_token = "new-account".into();
    f.store
        .lock()
        .await
        .unwrap()
        .write(Some(&replacement))
        .unwrap();
    refreshing.await.unwrap().unwrap();
    assert_eq!(
        f.service().access().await.unwrap().access_token,
        "new-account"
    );
}

#[tokio::test]
async fn failed_refresh_preserves_old_credentials_and_malformed_store_is_rejected() {
    let f = Fixture::new(true);
    f.store
        .lock()
        .await
        .unwrap()
        .write(Some(&credential(0)))
        .unwrap();
    assert!(f.service().access().await.is_err());
    assert_eq!(
        f.store
            .lock()
            .await
            .unwrap()
            .read()
            .unwrap()
            .unwrap()
            .refresh_token,
        "secret"
    );
    std::fs::write(f.directory.join("openai-codex.json"), "{secret-invalid").unwrap();
    assert_eq!(
        f.service().access().await.err().unwrap().to_string(),
        "Invalid Codex credential file"
    );
}

#[test]
fn cli_does_not_start_server_without_explicit_command() {
    assert!(matches!(cli::parse(&[]).unwrap(), cli::Command::Help));
    assert!(matches!(
        cli::parse(&["start".into()]).unwrap(),
        cli::Command::Start
    ));
    assert!(cli::parse(&["login".into(), "google".into()]).is_err());
}

#[tokio::test]
async fn worker_endpoint_returns_only_access_credentials_and_existing_failure_response() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let f = Fixture::new(false);
    let app = crate::api::router::create_worker_router(
        crate::adapters::worker_registry::WorkerRegistry::new(),
        Arc::new(crate::adapters::task_dispatcher::TaskDispatcher::new()),
    )
    .merge(router(f.service()));
    let unsupported = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/workers/auth/unknown-provider")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsupported.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(unsupported.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "Unsupported OAuth model provider");
    let request = || {
        Request::builder()
            .method("POST")
            .uri("/workers/auth/openai-codex")
            .body(Body::empty())
            .unwrap()
    };
    let response = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    f.store
        .lock()
        .await
        .unwrap()
        .write(Some(&credential(
            chrono::Utc::now().timestamp_millis() + 3_600_000,
        )))
        .unwrap();
    let response = app.oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["access_token"], "old");
    assert!(json.get("refresh_token").is_none());
}
