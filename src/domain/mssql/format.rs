use crate::services::config::DatabaseConfig;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MssqlFormat {
    Dacpac,
    Bacpac,
    Bak,
}

impl MssqlFormat {
    pub fn from_config(cfg: &DatabaseConfig) -> (Self, Option<String>) {
        match cfg.options.get("backup_format").and_then(|v| v.as_str()) {
            None | Some("dacpac") => (Self::Dacpac, None),
            Some("bacpac") => (Self::Bacpac, None),
            Some("bak") => (Self::Bak, None),
            Some(other) => (Self::Dacpac, Some(other.to_string())),
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "dacpac" => Some(Self::Dacpac),
            "bacpac" => Some(Self::Bacpac),
            "bak" => Some(Self::Bak),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Dacpac => ".dacpac",
            Self::Bacpac => ".bacpac",
            Self::Bak => ".bak",
        }
    }
}
