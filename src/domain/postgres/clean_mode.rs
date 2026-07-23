use crate::services::config::DatabaseConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreCleanMode {
    None,
    Clean,
    DropSchemas,
    DropDatabase,
}

impl RestoreCleanMode {
    pub fn from_config(cfg: &DatabaseConfig) -> (Self, Option<String>) {
        match cfg.options.get("clean_mode").and_then(|v| v.as_str()) {
            None | Some("clean") => (Self::Clean, None),
            Some("none") => (Self::None, None),
            Some("drop_schemas") => (Self::DropSchemas, None),
            Some("drop_database") => (Self::DropDatabase, None),
            Some(other) => (Self::Clean, Some(other.to_string())),
        }
    }

    pub fn uses_pg_restore_clean(self) -> bool {
        matches!(self, Self::Clean)
    }
}
