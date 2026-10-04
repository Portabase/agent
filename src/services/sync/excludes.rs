use crate::services::restic::excludes::bracket_negation;

/// P0 exclude patterns → `rclone sync --exclude` values. rclone filters are
/// relative to the sync root (spike 2026-10-03: 19/19 patterns give the P0 file set).
pub fn sync_excludes(patterns: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for raw in patterns {
        let pattern = raw.trim();
        let (anchored, rest) = match pattern.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, pattern),
        };
        let mut rest = rest.trim_end_matches('/');
        if !anchored {
            // Unanchored rclone patterns already match at any depth; a leading
            // `**/` would stop them from matching at the root.
            while let Some(stripped) = rest.strip_prefix("**/") {
                rest = stripped;
            }
        }
        if rest.is_empty() {
            continue;
        }
        let rest = bracket_negation(rest);
        // Collapse repeated /**/ into a single /**/
        let rest = collapse_double_stars(&rest);
        let full = if anchored { format!("/{rest}") } else { rest };
        for variant in zero_dir_variants(&full) {
            out.push(variant.clone());
            out.push(format!("{variant}/**"));
        }
    }
    out
}

/// Collapse repeated `/**/` sequences into a single `/**/`.
/// E.g., `a/**/**/b` becomes `a/**/b`.
fn collapse_double_stars(pattern: &str) -> String {
    let mut result = pattern.to_string();
    while result.contains("/**/**/") {
        result = result.replace("/**/**/", "/**/");
    }
    result
}

/// rclone's `**` between slashes needs at least one directory; globset's also
/// matches none. Emit every combination of each `/**/` kept or collapsed to `/`.
fn zero_dir_variants(pattern: &str) -> Vec<String> {
    let parts: Vec<&str> = pattern.split("/**/").collect();
    let gaps = parts.len() - 1;
    // ponytail: 2^gaps variants; past 4 gaps only "all kept" and "all collapsed".
    if gaps > 4 {
        return vec![pattern.to_string(), pattern.replace("/**/", "/")];
    }
    (0..1u32 << gaps)
        .map(|mask| {
            let mut variant = parts[0].to_string();
            for (i, part) in parts[1..].iter().enumerate() {
                variant.push_str(if mask & (1 << i) == 0 { "/**/" } else { "/" });
                variant.push_str(part);
            }
            variant
        })
        .collect()
}
