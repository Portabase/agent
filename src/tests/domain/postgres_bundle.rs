use crate::domain::postgres::bundle::{self, BundleManifest};
use crate::domain::postgres::PostgresDumpFormat;
use std::fs::File;
use std::io::Write;
use tempfile::TempDir;

#[test]
fn manifest_round_trips_through_write_and_read() {
    let dir = TempDir::new().unwrap();

    let manifest = BundleManifest {
        format: PostgresDumpFormat::Fc.as_str().to_string(),
        has_globals: true,
        dump_path: "dump.dump".to_string(),
    };
    manifest.write(dir.path()).unwrap();

    let read_back = BundleManifest::read(dir.path()).unwrap();
    assert_eq!(read_back, Some(manifest));
}

#[test]
fn manifest_read_returns_none_when_absent() {
    let dir = TempDir::new().unwrap();
    assert_eq!(BundleManifest::read(dir.path()).unwrap(), None);
}

#[test]
fn resolve_passes_through_a_plain_dump_file_unchanged() {
    let dir = TempDir::new().unwrap();
    let dump_file = dir.path().join("16678159.dump");
    File::create(&dump_file).unwrap().write_all(b"not a tarball").unwrap();

    let resolved = bundle::resolve(&dump_file).unwrap();

    assert_eq!(resolved.dump_path, dump_file);
    assert!(resolved.globals_path.is_none());
    assert!(resolved.format_override.is_none());
}

#[test]
fn resolve_passes_through_a_legacy_multi_file_tar_gz_unchanged() {
    let dir = TempDir::new().unwrap();
    let inner_dir = dir.path().join("inner");
    std::fs::create_dir_all(&inner_dir).unwrap();
    File::create(inner_dir.join("toc.dat")).unwrap().write_all(b"toc").unwrap();
    File::create(inner_dir.join("1234.dat.gz")).unwrap().write_all(b"data").unwrap();

    let tar_path = dir.path().join("legacy.tar.gz");
    let tar_gz = File::create(&tar_path).unwrap();
    let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
    let mut tar = tar::Builder::new(enc);
    tar.append_dir_all(".", &inner_dir).unwrap();
    tar.finish().unwrap();

    let resolved = bundle::resolve(&tar_path).unwrap();

    // No manifest.json inside this archive: bundle::resolve must defer to
    // restore.rs's own Fd unpack logic rather than touching it.
    assert_eq!(resolved.dump_path, tar_path);
    assert!(resolved.globals_path.is_none());
    assert!(resolved.format_override.is_none());
}
