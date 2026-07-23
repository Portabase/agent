use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use super::{prepare_archive, run_pg_restore, toc_creates_public_schema};
use crate::domain::postgres::clean_mode::RestoreCleanMode;
use crate::domain::postgres::connection::{
    can_drop_database, drop_all_schemas, drop_and_recreate_database, pg_restore_binary_name,
    recreate_public_schema, select_pg_path, server_version, terminate_connections,
};
use crate::domain::postgres::format::PostgresDumpFormat;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;

pub async fn run(
    cfg: DatabaseConfig,
    format: PostgresDumpFormat,
    restore_file: PathBuf,
    env: HashMap<String, String>,
    logger: Arc<JobLogger>,
) -> Result<()> {
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || -> Result<()> {
        logger.log("info", format!("Starting restore for database {}", cfg.name));

        let version = match handle.block_on(server_version(&cfg)) {
            Ok(v) => {
                logger.log("debug", format!("Postgres version detected: {}", v));
                v
            }
            Err(e) => {
                logger.log("error", format!("Failed to get server version for {}: {:?}", cfg.name, e));
                return Err(e.into());
            }
        };

        let pg_restore = select_pg_path(&version).join(pg_restore_binary_name());

        logger.log("debug", format!("Using pg_restore at {:?}", pg_restore));

        let keep_ownership = cfg.options
            .get("keep_ownership")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if keep_ownership {
            logger.log("info", format!("Restoring ownership and privileges for {}", cfg.name));
        } else {
            logger.log("info", format!("Stripping ownership and privileges for {} (--no-owner --no-privileges)", cfg.name));
        }

        let (mode, bad_value) = RestoreCleanMode::from_config(&cfg);
        if let Some(v) = bad_value {
            logger.log("warn", format!("Unknown clean_mode '{}' for {}, falling back to 'clean'", v, cfg.name));
        }

        let prepared = prepare_archive(format, &restore_file, &pg_restore, &logger)?;

        match mode {
            RestoreCleanMode::DropSchemas => {
                handle.block_on(terminate_connections(&cfg))?;
                let owner = cfg.username.clone();
                let dropped = handle.block_on(drop_all_schemas(&cfg))?;
                logger.log("warn", format!("clean_mode=drop_schemas dropped schemas {:?} in {}", dropped, cfg.database));
                if !toc_creates_public_schema(prepared.toc()) {
                    handle.block_on(recreate_public_schema(&cfg, &owner))?;
                }
            }
            RestoreCleanMode::DropDatabase => {
                if !handle.block_on(can_drop_database(&cfg))? {
                    anyhow::bail!(
                        "clean_mode=drop_database requires CREATEDB + ownership on {}; use clean_mode=drop_schemas instead",
                        cfg.database
                    );
                }
                logger.log("warn", format!("clean_mode=drop_database DROPPING database {} before restore", cfg.database));
                handle.block_on(drop_and_recreate_database(&cfg))?;
            }
            RestoreCleanMode::Clean | RestoreCleanMode::None => {
                handle.block_on(terminate_connections(&cfg))?;
            }
        }

        let mut cmd = Command::new(&pg_restore);
        if !keep_ownership {
            cmd.args(["--no-owner", "--no-privileges"]);
        }
        if mode.uses_pg_restore_clean() {
            cmd.args(["--clean", "--if-exists"]);
        }
        cmd.arg("--host").arg(&cfg.host)
            .arg("--port").arg(cfg.port.to_string())
            .arg("--username").arg(&cfg.username)
            .arg("--dbname").arg(&cfg.database)
            .arg("-v");
        if matches!(format, PostgresDumpFormat::Fd) {
            cmd.arg("-j").arg("4");
        }
        cmd.arg(prepared.path()).envs(env);

        run_pg_restore(cmd, &logger, &cfg)?;
        logger.log("info", format!("Restore finished for database {}", cfg.name));
        Ok(())
    })
    .await?
}
