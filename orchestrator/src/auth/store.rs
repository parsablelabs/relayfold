use super::OAuthCredential;
use anyhow::Context;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
};
use tokio::time::{Duration, Instant, sleep};

#[derive(Clone)]
pub struct FileCredentialStore {
    directory: PathBuf,
}

pub struct LockedStore {
    directory: PathBuf,
    _lock: File,
}

impl FileCredentialStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn from_environment() -> anyhow::Result<Self> {
        let directory = match std::env::var_os("RELAYFOLD_OAUTH_DIR") {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(
                std::env::var_os("HOME").context("HOME or RELAYFOLD_OAUTH_DIR is required")?,
            )
            .join(".relayfold/oauth"),
        };
        Ok(Self::new(directory))
    }

    pub async fn lock(&self) -> anyhow::Result<LockedStore> {
        std::fs::create_dir_all(&self.directory).context("Unable to create OAuth directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        // Stable lock file: token-file rename must not replace the lock inode.
        let file = options
            .open(self.directory.join("openai-codex.lock"))
            .context("Unable to open OAuth lock")?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(LockedStore {
                        directory: self.directory.clone(),
                        _lock: file,
                    });
                }
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    sleep(Duration::from_millis(25)).await
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    anyhow::bail!("Timed out waiting for OAuth credential lock")
                }
                Err(_) => anyhow::bail!("Unable to lock OAuth credentials"),
            }
        }
    }
}

impl LockedStore {
    pub fn read(&self) -> anyhow::Result<Option<OAuthCredential>> {
        let bytes = match std::fs::read(self.directory.join("openai-codex.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => anyhow::bail!("Unable to read Codex credentials"),
        };
        let credential: OAuthCredential = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid Codex credential file"))?;
        anyhow::ensure!(
            !credential.access_token.is_empty()
                && !credential.refresh_token.is_empty()
                && !credential.account_id.is_empty()
                && credential.token_type.eq_ignore_ascii_case("bearer"),
            "Invalid Codex credential file"
        );
        Ok(Some(credential))
    }

    pub fn write(&self, credential: Option<&OAuthCredential>) -> anyhow::Result<()> {
        let destination = self.directory.join("openai-codex.json");
        if let Some(credential) = credential {
            let temporary = self.directory.join("openai-codex.json.tmp");
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temporary)
                .context("Unable to write Codex credentials")?;
            file.write_all(&serde_json::to_vec(credential)?)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &destination)
                .context("Unable to persist Codex credentials")?;
        } else {
            match std::fs::remove_file(destination) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => anyhow::bail!("Unable to remove Codex credentials"),
            }
        }
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}
