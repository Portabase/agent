use anyhow::{Context, Result};
use chrono::Local;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::fs::{OpenOptions, create_dir_all, metadata, read_dir, remove_file};
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};

/// Lock type for logging purposes
#[derive(Debug, Copy, Clone)]
pub enum DbOpLock {
    Backup,
    Restore,
}

impl DbOpLock {
    /// Convert enum to a static string for service name
    pub fn as_str(&self) -> &'static str {
        match self {
            DbOpLock::Backup => "backup-service",
            DbOpLock::Restore => "restore-service",
        }
    }
}

/// File-based lock utility
pub struct FileLock;

impl FileLock {
    const LOCK_DIR: &'static str = "/var/locks";

    /// Returns the path for the lock file
    fn lock_file_path(id: &str) -> PathBuf {
        Path::new(Self::LOCK_DIR).join(format!("{}.lock", id))
    }

    /// Ensure the locks directory exists
    async fn ensure_lock_dir() -> Result<()> {
        create_dir_all(Self::LOCK_DIR)
            .await
            .with_context(|| format!("Failed to create locks directory {}", Self::LOCK_DIR))?;
        Ok(())
    }

    /// Clean all lock files on startup (remove everything, ignore stale)
    pub async fn clean_startup() -> Result<()> {
        Self::ensure_lock_dir().await?;
        let mut dir = read_dir(Self::LOCK_DIR).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().map(|e| e == "lock").unwrap_or(false) {
                if let Err(e) = remove_file(&path).await {
                    warn!("Failed to remove lock file {:?}: {:?}", path, e);
                } else {
                    info!("Removed lock file {:?}", path);
                }
            }
        }
        Ok(())
    }

    pub async fn is_locked(id: &str) -> Result<bool> {
        let path = Self::lock_file_path(id);

        if !path.exists() {
            return Ok(false);
        }

        let meta = metadata(&path).await?;

        if let Ok(modified) = meta.modified() {
            let age = SystemTime::now().duration_since(modified)?;

            if age > Duration::from_secs(24 * 60 * 60) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Acquire a file-based lock
    pub async fn acquire(id: &str, service_name: &str) -> Result<()> {
        Self::ensure_lock_dir().await?;

        let path = Self::lock_file_path(id);
        info!("Attempting to acquire lock for {} at {:?}", id, path);

        if path.exists() {
            let meta = metadata(&path).await?;
            if let Ok(modified) = meta.modified() {
                let age = SystemTime::now().duration_since(modified)?;
                if age > Duration::from_secs(24 * 60 * 60) {
                    remove_file(&path).await?;
                    warn!("Removed stale lock for {}", id);
                } else {
                    error!("Lock already held for {}. Cannot acquire.", id);
                    anyhow::bail!("backup_already_in_progress");
                }
            }
        }

        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .with_context(|| format!("Failed to create lock file for {}", id))?;

        f.write_all(format!("Service: {}\n", service_name).as_bytes())
            .await?;
        f.write_all(format!("PID: {}\n", std::process::id()).as_bytes())
            .await?;
        // f.write_all(format!("Timestamp: {}\n", Utc::now()).as_bytes()).await?;
        f.write_all(format!("Timestamp: {}\n", Local::now()).as_bytes())
            .await?;

        info!("Successfully acquired lock for {}", id);
        Ok(())
    }

    /// Release the file-based lock
    pub async fn release(id: &str) -> Result<()> {
        let path = Self::lock_file_path(id);

        info!("Releasing lock for {}", id);
        if path.exists() {
            remove_file(&path).await?;
            info!("Released file lock for {}", id);
        } else {
            warn!(
                "Attempted to release lock for {}, but file does not exist",
                id
            );
        }
        Ok(())
    }
}
