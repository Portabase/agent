#![allow(dead_code)]

use crate::core::context::Context;
use crate::services::status::DatabaseStatus;
use crate::utils::common::vec_to_option_json;
use crate::utils::redis_client;
use crate::utils::task_manager::cron::check_and_update_cron;
use redis::aio::MultiplexedConnection;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct CronService {
    ctx: Arc<Context>,
    conn: MultiplexedConnection,
}

impl CronService {
    pub async fn new(ctx: Arc<Context>) -> Self {
        let conn = redis_client::redis_connection().await;
        CronService { ctx, conn }
    }

    pub async fn sync(&mut self, database: &DatabaseStatus) -> Result<bool, String> {
        let generated_id = database.generated_id.as_str();
        let dbms = database.dbms.as_str();
        let task_name = format!("periodic.backup_{}", generated_id);
        let args = vec![generated_id.to_string(), dbms.to_string()];
        let storages: Option<Value> = vec_to_option_json(database.storages.clone());
        let metadata = json!({
            "storages": storages
        });

        check_and_update_cron(
            &mut self.conn,
            database.data.backup.cron.clone(),
            args,
            "tasks.database.periodic_backup",
            task_name,
            Option::from(metadata),
        )
        .await;

        Ok(true)
    }
}
