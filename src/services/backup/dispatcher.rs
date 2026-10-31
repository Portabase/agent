use super::service::BackupService;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::config::DatabasesConfig;
use crate::settings::CONFIG;
use crate::utils::common::BackupMethod;
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::future::Future;
use std::sync::Mutex;
use tokio::sync::Semaphore;
use tracing::{error, warn};

pub(crate) static BACKUP_SEMAPHORE: Lazy<Option<Semaphore>> =
    Lazy::new(|| CONFIG.max_concurrent_backups.map(Semaphore::new));

static IN_FLIGHT_AUTOMATIC: Lazy<Mutex<HashSet<String>>> = Lazy::new(Default::default);

pub(crate) struct InFlightBackup(String);

impl InFlightBackup {
    pub(crate) fn claim(generated_id: &str) -> Option<Self> {
        IN_FLIGHT_AUTOMATIC
            .lock()
            .unwrap()
            .insert(generated_id.to_string())
            .then(|| Self(generated_id.to_string()))
    }
}

impl Drop for InFlightBackup {
    fn drop(&mut self) {
        IN_FLIGHT_AUTOMATIC.lock().unwrap().remove(&self.0);
    }
}

pub(crate) async fn run_with_permit<F>(semaphore: Option<&Semaphore>, job: F)
where
    F: Future<Output = ()>,
{
    let _permit = match semaphore {
        Some(semaphore) => semaphore.acquire().await.ok(),
        None => None,
    };

    job.await;
}

impl BackupService {
    pub async fn dispatch(
        &self,
        generated_id: &String,
        config: &DatabasesConfig,
        method: BackupMethod,
        storages: &Vec<DatabaseStorage>,
        encrypt: bool,
        engine: &str,
    ) {
        let Some(cfg) = config
            .databases
            .iter()
            .find(|c| c.generated_id == generated_id.as_str())
        else {
            error!("Database config not found for {}", generated_id);
            return;
        };

        // Manual backups are already marked ongoing by the dashboard, so they always run.
        let in_flight = match method {
            BackupMethod::Automatic => match InFlightBackup::claim(generated_id) {
                Some(guard) => Some(guard),
                None => {
                    warn!(
                        "Automatic backup for {} already queued or running; skipping",
                        generated_id
                    );
                    return;
                }
            },
            BackupMethod::Manual => None,
        };

        let service = Self {
            ctx: self.ctx.clone(),
        };

        let db_cfg = cfg.clone();
        let storages = storages.clone();
        let generated_id = generated_id.clone();
        let engine = engine.to_string();

        tokio::spawn(async move {
            let _in_flight = in_flight;
            run_with_permit(BACKUP_SEMAPHORE.as_ref(), async move {
                if let Err(e) = service
                    .execute_backup(generated_id, db_cfg, method, storages, encrypt, engine)
                    .await
                {
                    error!("Backup execution failed: {}", e);
                }
            })
            .await;
        });
    }
}
