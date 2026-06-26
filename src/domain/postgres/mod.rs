pub mod backup;
pub(crate) mod connection;
pub mod database;
mod format;
pub(crate) mod globals;
mod ping;
mod restore;

pub use connection::{detect_format_from_file, detect_format_from_size};
pub use format::PostgresDumpFormat;
