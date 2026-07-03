mod core;
mod domain;
mod services;
mod settings;
mod tasks;
#[cfg(test)]
mod tests;
mod utils;

use crate::tasks::ping::ping_server;
use crate::utils::locks::FileLock;
use crate::utils::logging;
use utils::redis_client;
use utils::task_manager::scheduler;

#[tokio::main]
async fn main() {
    logging::init_logger();

    // Remove all locks on startup
    if let Err(e) = FileLock::clean_startup().await {
        eprintln!("Failed to clean locks on startup: {:?}", e);
    }

    // Best-effort cleanup of ephemeral helper containers orphaned by a crash.
    match crate::domain::docker_volume::docker::client() {
        Ok(docker) => match crate::domain::docker_volume::docker::sweep_ephemeral(&docker).await {
            Ok(n) if n > 0 => tracing::info!("Removed {n} orphaned ephemeral helper container(s)"),
            Ok(_) => {}
            Err(e) => tracing::warn!("Ephemeral helper sweep failed: {e}"),
        },
        Err(e) => tracing::debug!("Docker socket unavailable, skipping helper sweep: {e}"),
    }

    tokio::join!(ping_server(), async {
        let conn = redis_client::redis_connection().await;
        scheduler::scheduler_loop(conn).await;
    });
}
