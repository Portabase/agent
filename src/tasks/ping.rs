use crate::core::agent::Agent;
use crate::core::context::Context;
use crate::settings::CONFIG;
use crate::utils::common::BackupMethod;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};

pub async fn ping_server() {
    let ctx = Arc::new(Context::new());
    let mut agent = Agent::new(ctx.clone()).await;

    loop {
        info!("Ping server task started");

        if let Err(e) = agent.run(BackupMethod::Manual).await {
            error!(
                "An error occurred while executing task ping_server: {:?}",
                e
            );
        }
        tokio::time::sleep(Duration::from_secs(CONFIG.pooling as u64)).await;
    }
}