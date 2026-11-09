use crate::domain::postgres::connection::connect;
use crate::services::config::DatabaseConfig;
use anyhow::{Result, bail};

#[derive(Debug)]
pub struct ServerFacts {
    pub major: u32,
    pub system_identifier: Option<u64>,
}

pub async fn preflight(cfg: &DatabaseConfig) -> Result<ServerFacts> {
    let client = connect(cfg).await?;
    let version = client
        .query_one(
            "SELECT current_setting('server_version_num')::int, current_setting('server_version')",
            &[],
        )
        .await?;
    let version_num: i32 = version.try_get(0)?;
    if version_num < 170000 {
        bail!(
            "Physical backups need PostgreSQL 17 or later (server is {}). Use the Archive method for this source.",
            version.try_get::<_, String>(1)?
        );
    }

    let requirements = client
        .query_one(
            "SELECT rolreplication OR rolsuper, current_setting('summarize_wal')::bool, \
             (SELECT string_agg(spcname, ', ' ORDER BY spcname) FROM pg_tablespace \
              WHERE spcname NOT IN ('pg_default', 'pg_global')) \
             FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await?;
    if !requirements.try_get::<_, bool>(0)? {
        bail!(
            "The role {user} lacks the REPLICATION privilege. Run: ALTER ROLE {user} WITH REPLICATION;",
            user = cfg.username
        );
    }
    if !requirements.try_get::<_, bool>(1)? {
        bail!(
            "WAL summarization is off. Run: ALTER SYSTEM SET summarize_wal = on; SELECT pg_reload_conf(); (managed PostgreSQL: set the summarize_wal parameter)"
        );
    }
    if let Some(names) = requirements.try_get::<_, Option<String>>(2)? {
        bail!(
            "Custom tablespaces are not supported by physical backups ({names}). Drop them or use the Archive method."
        );
    }

    let system_identifier = client
        .query_one("SELECT system_identifier FROM pg_control_system()", &[])
        .await
        .ok()
        .and_then(|row| row.try_get::<_, i64>(0).ok())
        .map(|identifier| identifier as u64);

    Ok(ServerFacts {
        major: (version_num / 10000) as u32,
        system_identifier,
    })
}
