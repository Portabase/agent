use anyhow::{Result, bail};
use std::path::Path;

/// Escapes restic/filepath.Match metacharacters so a literal path matches only itself.
pub fn glob_escape(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    for c in literal.chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// globset (the P0 matcher) reads `[!x]` as bracket negation; restic (Go
/// `filepath.Match`) and rclone only know `[^x]` and take `!` literally.
pub(crate) fn bracket_negation(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        out.push(c);
        match c {
            '\\' => out.extend(chars.next()),
            '[' if !in_class => {
                in_class = true;
                if chars.as_str().starts_with('!') {
                    chars.next();
                    out.push('^');
                }
            }
            ']' => in_class = false,
            _ => {}
        }
    }
    out
}

/// P0 patterns, trimmed like the P0 matcher, as `(anchored, pattern)`; a leading `/` marks anchored ones.
fn normalized(patterns: &[String]) -> Result<Vec<(bool, String)>> {
    let mut out = Vec::new();
    for raw in patterns {
        let pattern = raw.trim();
        if pattern.contains(['{', '}']) {
            bail!("exclude pattern '{pattern}': brace patterns are not supported in Snapshots mode");
        }
        let (anchored, rest) = match pattern.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, pattern),
        };
        let rest = rest.trim_end_matches('/');
        if !rest.is_empty() {
            out.push((anchored, bracket_negation(rest)));
        }
    }
    Ok(out)
}

/// `restic backup` matches patterns against absolute paths, so every pattern is
/// rooted at the source; unanchored ones would otherwise also match directories
/// above it and exclude the whole source (spike S5).
pub fn backup_excludes(root: &Path, patterns: &[String]) -> Result<Vec<String>> {
    let root = glob_escape(root.to_string_lossy().trim_end_matches('/'));
    Ok(normalized(patterns)?
        .into_iter()
        .map(|(anchored, p)| {
            if anchored { format!("{root}/{p}") } else { format!("{root}/**/{p}") }
        })
        .collect())
}

/// `restic restore <id>:<path>` matches patterns relative to that subfolder (spike S1).
pub fn restore_excludes(patterns: &[String]) -> Result<Vec<String>> {
    Ok(normalized(patterns)?
        .into_iter()
        .map(|(anchored, p)| if anchored { format!("/{p}") } else { format!("/**/{p}") })
        .collect())
}
