use crate::services::restore::downloader::{BYTE_STEP, ProgressTracker};

const MB: usize = 1024 * 1024;

#[test]
fn percent_milestones_small_chunks() {
    // total = 1000 MB, fed 100 MB at a time => one milestone per advance, 10 total.
    let total = 1000u64 * 1024 * 1024;
    let mut t = ProgressTracker::new(Some(total));

    let mut all = Vec::new();
    for _ in 0..10 {
        all.extend(t.advance(100 * MB));
    }

    assert_eq!(all.len(), 10);
    assert!(all[0].starts_with("Download progress: 10%"));
    assert_eq!(all[2], "Download progress: 30% (300/1000 MB)");
    assert!(all[9].starts_with("Download progress: 100%"));
}

#[test]
fn percent_single_chunk_crosses_multiple() {
    let total = 100u64 * 1024 * 1024;
    let mut t = ProgressTracker::new(Some(total));

    let msgs = t.advance(35 * MB); // jump to 35%
    assert_eq!(msgs.len(), 3);
    assert!(msgs[0].starts_with("Download progress: 10%"));
    assert!(msgs[1].starts_with("Download progress: 20%"));
    assert!(msgs[2].starts_with("Download progress: 30%"));
}

#[test]
fn percent_caps_at_100() {
    // downloaded exceeds total (e.g. compressed transfer): never log past 100%.
    let total = 100u64 * 1024 * 1024;
    let mut t = ProgressTracker::new(Some(total));

    let msgs = t.advance(200 * MB);
    assert_eq!(msgs.len(), 10); // 10..=100
    assert_eq!(*msgs.last().unwrap(), "Download progress: 100% (200/100 MB)");
    assert_eq!(msgs.iter().filter(|m| m.contains("110%")).count(), 0);
}

#[test]
fn byte_mode_when_total_unknown() {
    let mut t = ProgressTracker::new(None);

    assert!(t.advance(100 * MB).is_empty()); // below first 512 MB step
    let msgs = t.advance(500 * MB); // crosses 512 MB at downloaded = 600 MB
    assert_eq!(msgs, vec!["Downloaded 600 MB".to_string()]);

    let _ = BYTE_STEP; // ensure the const is exported
}

#[test]
fn zero_total_no_progress_no_panic() {
    let mut t = ProgressTracker::new(Some(0));
    assert!(t.advance(10 * MB).is_empty());
}

#[test]
fn fmt_total_variants() {
    assert_eq!(ProgressTracker::fmt_total(Some(1024 * 1024 * 1024)), "1024 MB");
    assert_eq!(ProgressTracker::fmt_total(None), "unknown size");
    assert_eq!(ProgressTracker::fmt_total(Some(0)), "unknown size");
}
