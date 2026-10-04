use crate::services::config::DatabaseConfig;
use anyhow::Result;
use std::path::Path;
use tokio::time::{Duration, timeout};

pub async fn run(cfg: DatabaseConfig) -> Result<bool> {
    // A hung mount (NFS) blocks is_dir/read_dir: keep it off the runtime and bounded.
    let check = tokio::task::spawn_blocking(move || {
        let root = Path::new(&cfg.path);
        root.is_dir() && std::fs::read_dir(root).is_ok()
    });
    Ok(matches!(
        timeout(Duration::from_secs(10), check).await,
        Ok(Ok(true))
    ))
}
