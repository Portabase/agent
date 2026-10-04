//! Unix-only metadata behind `cfg`, with inert fallbacks so the Windows release
//! build compiles.

/// Device id, used to spot mount points. Off unix every entry reports 0, so no
/// directory is ever treated as a mount point.
#[cfg(unix)]
pub fn dev(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.dev()
}

#[cfg(not(unix))]
pub fn dev(_meta: &std::fs::Metadata) -> u64 {
    0
}

/// Whether the agent can restore file ownership.
#[cfg(unix)]
pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
pub fn is_root() -> bool {
    false
}
