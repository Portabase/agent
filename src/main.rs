mod core;
mod domain;
mod services;
mod settings;
mod tasks;
mod utils;
#[cfg(test)]
mod tests;

use crate::tasks::ping::ping_server;
use crate::utils::locks::FileLock;
use utils::redis_client;
use utils::task_manager::scheduler;
use crate::utils::logging;

#[tokio::main]
async fn main() {

    logging::init_logger();

    // Remove all locks on startup
    if let Err(e) = FileLock::clean_startup().await {
        eprintln!("Failed to clean locks on startup: {:?}", e);
    }

    tokio::join!(ping_server(), async {
        let conn = redis_client::redis_connection().await;
        scheduler::scheduler_loop(conn).await;
    });
}
