use super::{
    codex::{CodexProvider, LoginAttempt, REDIRECT_URI},
    store::FileCredentialStore,
};
use axum::{
    Router,
    extract::{OriginalUri, State},
    http::StatusCode,
    routing::get,
};
use std::{io::Write, sync::Arc};
use tokio::sync::{Mutex, oneshot};

pub const HELP: &str = "RelayFold orchestrator\n\nUsage: orchestrator <COMMAND>\n\nCommands:\n  start                    Start the orchestrator server\n  login [openai-codex]      Connect the installation's Codex account\n  logout [openai-codex]     Remove local Codex credentials\n  auth-status              Show Codex connection status\n\nOAuth storage: RELAYFOLD_OAUTH_DIR (default: ~/.relayfold/oauth)\n";

pub enum Command {
    Help,
    Start,
    Login,
    Logout,
    Status,
}

pub fn parse(args: &[String]) -> anyhow::Result<Command> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["help"] | ["--help"] | ["-h"] => Ok(Command::Help),
        ["start"] => Ok(Command::Start),
        ["login"] | ["login", "openai-codex"] => Ok(Command::Login),
        ["logout"] | ["logout", "openai-codex"] => Ok(Command::Logout),
        ["auth-status"] => Ok(Command::Status),
        _ => anyhow::bail!("Unknown command or provider.\n{HELP}"),
    }
}

pub async fn run(command: Command) -> anyhow::Result<()> {
    let store = FileCredentialStore::from_environment()?;
    match command {
        Command::Login => {
            let attempt = Arc::new(LoginAttempt::new());
            let code = authorization_code(attempt.clone()).await?;
            let credential = CodexProvider::new()?.exchange(&attempt, &code).await?;
            store.lock().await?.write(Some(&credential))?;
            println!("Codex connected for this installation.");
        }
        Command::Logout => {
            store.lock().await?.write(None)?;
            println!("Codex credentials removed locally. Already-issued tokens are not revoked.");
        }
        Command::Status => {
            let connected = store.lock().await?.read()?.is_some();
            println!(
                "Codex: {}",
                if connected {
                    "credentials stored"
                } else {
                    "not connected"
                }
            );
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[derive(Clone)]
struct Callback {
    attempt: Arc<LoginAttempt>,
    sender: Arc<Mutex<Option<oneshot::Sender<String>>>>,
}

async fn callback(
    State(state): State<Callback>,
    OriginalUri(uri): OriginalUri,
) -> (StatusCode, &'static str) {
    let input = format!(
        "{}{}",
        REDIRECT_URI,
        uri.query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default()
    );
    match state.attempt.code(&input) {
        Ok(code) => {
            if let Some(sender) = state.sender.lock().await.take() {
                let _ = sender.send(code);
            }
            (
                StatusCode::OK,
                "Authorization received. Return to the RelayFold terminal.",
            )
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            "Invalid authorization callback. Retry login in the terminal.",
        ),
    }
}

async fn authorization_code(attempt: Arc<LoginAttempt>) -> anyhow::Result<String> {
    let (sender, callback_input) = oneshot::channel();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:1455").await;
    let server = listener.ok().map(|listener| {
        let state = Callback {
            attempt: attempt.clone(),
            sender: Arc::new(Mutex::new(Some(sender))),
        };
        tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                Router::new()
                    .route("/auth/callback", get(callback))
                    .with_state(state),
            )
            .await;
        })
    });
    println!("Open this URL in your browser:\n\n{}\n", attempt.url);
    println!(
        "Waiting for the callback, or paste the full redirect URL here (including code and state).\nOn a remote server the browser's localhost redirect may fail; copy its URL anyway.\nPress Ctrl-C to cancel."
    );
    std::io::stdout().flush()?;
    let (manual_sender, manual_input) = oneshot::channel();
    // A detached thread avoids tokio stdin's uncancellable blocking read during
    // runtime shutdown when the browser callback completes first.
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = std::io::stdin()
            .read_line(&mut line)
            .map(|count| (count, line));
        let _ = manual_sender.send(result);
    });
    let result = tokio::select! {
        result = callback_input, if server.is_some() => result.map_err(|_| anyhow::anyhow!("OAuth callback closed")),
        result = manual_input => {
            match result {
                Ok(Ok((count, line))) if count > 0 => attempt.code(&line),
                _ => Err(anyhow::anyhow!("Login cancelled: no redirect URL supplied")),
            }
        },
        _ = tokio::signal::ctrl_c() => Err(anyhow::anyhow!("Login cancelled")),
        _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => Err(anyhow::anyhow!("Login timed out; start a new login attempt")),
    };
    if let Some(server) = server {
        server.abort();
    }
    result
}
