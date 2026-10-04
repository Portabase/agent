use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};

/// Exclude patterns, matched against paths relative to the source root.
/// No leading `/` matches at any depth; a leading `/` anchors at the root.
pub struct ExcludeMatcher {
    set: GlobSet,
    literal: Vec<PathBuf>,
}

impl ExcludeMatcher {
    pub fn new(patterns: &[String]) -> Result<Self> {
        let mut builder = GlobSetBuilder::new();
        for raw in patterns {
            let pattern = raw.trim();
            if pattern.is_empty() {
                continue;
            }
            let glob = match pattern.strip_prefix('/') {
                Some(anchored) => anchored.trim_end_matches('/').to_string(),
                None => format!("**/{}", pattern.trim_end_matches('/')),
            };
            builder.add(
                GlobBuilder::new(&glob)
                    .literal_separator(true)
                    .build()
                    .with_context(|| format!("invalid exclude pattern '{pattern}'"))?,
            );
        }
        Ok(Self {
            set: builder.build()?,
            literal: Vec::new(),
        })
    }

    pub fn from_config(cfg: &DatabaseConfig) -> Result<Self> {
        Self::new(&exclude_patterns(cfg))
    }

    /// Also exclude exactly this root-relative path (and its subtree), compared
    /// literally so glob characters in the name mean nothing.
    pub fn exclude_path(&mut self, relative: PathBuf) {
        self.literal.push(relative);
    }

    pub fn is_excluded(&self, relative: &Path) -> bool {
        self.set.is_match(relative) || self.literal.iter().any(|p| p == relative)
    }
}

/// The `options.exclude` patterns of a files source, as configured.
pub fn exclude_patterns(cfg: &DatabaseConfig) -> Vec<String> {
    cfg.options
        .get("exclude")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `options.one_file_system` of a files source.
pub fn one_file_system(cfg: &DatabaseConfig) -> bool {
    cfg.options
        .get("one_file_system")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
