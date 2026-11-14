use super::models::{DatabaseConfig, DbType, InputDatabaseConfig};
use super::options::validate_files_options;
use uuid::Uuid;

fn required<T: Clone>(opt: &Option<T>, db_name: &str, field_name: &str) -> Result<T, String> {
    match opt {
        Some(v) => Ok(v.clone()),
        None => Err(format!(
            "Missing required field '{}' for database '{}'",
            field_name, db_name
        )),
    }
}

fn optional<T: Clone + Default>(opt: &Option<T>) -> T {
    opt.clone().unwrap_or_default()
}

pub fn build_config(db: InputDatabaseConfig) -> Result<DatabaseConfig, String> {
    if Uuid::parse_str(&db.generated_id).is_err() {
        return Err(format!("Invalid UUID for database '{}'", db.name));
    }

    let username = match db.db_type {
        DbType::Postgresql
        | DbType::PostgresqlCluster
        | DbType::Mysql
        | DbType::Mariadb
        | DbType::Mssql => required(&db.username, &db.name, "username")?,
        _ => optional(&db.username),
    };
    let password = match db.db_type {
        DbType::Postgresql
        | DbType::PostgresqlCluster
        | DbType::Mysql
        | DbType::Mariadb
        | DbType::Mssql => required(&db.password, &db.name, "password")?,
        _ => optional(&db.password),
    };
    let host = match db.db_type {
        DbType::Postgresql
        | DbType::PostgresqlCluster
        | DbType::Mysql
        | DbType::Mariadb
        | DbType::MongoDB
        | DbType::Redis
        | DbType::Firebird
        | DbType::Valkey
        | DbType::Mssql => required(&db.host, &db.name, "host")?,
        DbType::Sqlite | DbType::DockerVolume | DbType::Files => optional(&db.host),
    };

    let port = match db.db_type {
        DbType::Postgresql
        | DbType::PostgresqlCluster
        | DbType::Mysql
        | DbType::Mariadb
        | DbType::Redis
        | DbType::Firebird
        | DbType::Valkey
        | DbType::Mssql => required(&db.port, &db.name, "port")?,
        DbType::MongoDB | DbType::Sqlite | DbType::DockerVolume | DbType::Files => {
            db.port.unwrap_or(0)
        }
    };

    let database_name = match db.db_type {
        DbType::Sqlite | DbType::Redis | DbType::Valkey | DbType::DockerVolume | DbType::Files => {
            optional(&db.database)
        }
        DbType::PostgresqlCluster => db
            .database
            .clone()
            .unwrap_or_else(|| "postgres".to_string()),
        _ => required(&db.database, &db.name, "database")?,
    };
    let path_val = match db.db_type {
        DbType::Sqlite => required(&db.path, &db.name, "path")?,
        DbType::Files => {
            let p = required(&db.path, &db.name, "path")?;
            if !p.starts_with('/') || p.trim_end_matches('/').is_empty() {
                return Err(format!(
                    "Path for files source '{}' must be absolute and not '/'",
                    db.name
                ));
            }
            p
        }
        _ => optional(&db.path),
    };
    if matches!(db.db_type, DbType::Files) {
        validate_files_options(&db.name, db.options.as_ref())?;
    }
    let max_packet_size = match db.db_type {
        DbType::Mysql | DbType::Mariadb => db.max_packet_size.unwrap_or_else(|| "512M".to_string()),
        _ => String::new(),
    };
    let volume_name = match db.db_type {
        DbType::DockerVolume => required(&db.volume_name, &db.name, "volume_name")?,
        _ => optional(&db.volume_name),
    };
    if let Some(hooks) = &db.hooks {
        hooks.validate(&db.name)?;
    }

    Ok(DatabaseConfig {
        name: db.name,
        database: database_name,
        db_type: db.db_type,
        username,
        password,
        host,
        port,
        generated_id: db.generated_id,
        path: path_val,
        max_packet_size,
        volume_name,
        container_name: db.container_name.clone(),
        options: db.options.unwrap_or_default(),
        hooks: db.hooks,
    })
}
