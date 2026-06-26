pub mod backup;
pub(crate) mod bundle;
pub(crate) mod connection;
pub mod database;
mod format;
pub(crate) mod globals;
mod ping;
mod restore;

pub use connection::{detect_format_from_file, detect_format_from_size};
// Re-exported for the in-crate test suite (`tests::domain::postgres_bundle`);
// production code reaches the type via `super::format::PostgresDumpFormat`,
// so the re-export is unused in a non-test build.
#[cfg_attr(not(test), allow(unused_imports))]
pub use format::PostgresDumpFormat;
