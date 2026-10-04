//! Sync mode: a destination kept identical to a files source (`rclone sync`).
//! Replication, not backup: deletions propagate.
pub mod backup;
pub mod excludes;
pub mod stats;
pub mod rclone;
