//! Geodata readiness for the mihomo core, shared verbatim by the GUI and the
//! elevated service (`#[path]` include; std-only, no extra dependencies).
//!
//! The core downloads `geoip.metadb` / `geosite.dat` on first start and may be
//! killed mid-download, leaving a truncated MMDB behind. mihomo then either
//! refuses to boot or boots and silently loses geo rules, which used to look
//! like a hanging TUN start with a short 8 s readiness budget.
//!
//! Preparation runs *before* the core is spawned,
//! with the core already stopped, on the application's own data directory (the
//! GUI's app data dir, or the service's protected data dir). It:
//!
//!   * rejects a data directory that is a symlink / Windows reparse point or
//!     not a real directory (fail closed, never delete user data),
//!   * sanity-checks the two standard MMDB names (`geoip.metadb`,
//!     `Country.mmdb`) with a bounded tail read and quarantines a suspicious
//!     file *inside the same directory* under a unique `.incomplete` name,
//!   * reports whether the next start is "cold" (geodata incomplete, so the
//!     core will have to fetch or download something) or "warm".
//!
//! Honesty about the check: looking for the trailing MMDB marker plus metadata
//! bytes is a **truncation sanity check, not full integrity validation**. A
//! corrupted-but-well-formed file passes; a file whose metadata region is
//! larger than the 128 KiB tail window could be misjudged (real MMDB metadata
//! is a few hundred bytes, so this is a theoretical concern only).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Standard MMDB names to inspect; only the default cache proves a warm start.
pub const MMDB_NAMES: [&str; 2] = ["geoip.metadb", "Country.mmdb"];
/// Standard geosite file names (exact case: `GeoSite.dat` is the legacy name).
pub const GEOSITE_NAMES: [&str; 2] = ["geosite.dat", "GeoSite.dat"];
pub const BUNDLED_NAMES: [&str; 4] = [
    "geoip.metadb", "geosite.dat", "geoip.dat", "ASN.mmdb",
];

pub fn bundled_sources(dir: &Path) -> Vec<(&'static str, PathBuf)> {
    BUNDLED_NAMES.iter().map(|&name| (name, dir.join(name))).collect()
}

pub fn validate_bundled(sources: &[(&str, PathBuf)]) -> Result<(), String> {
    for (name, source) in sources {
        if !BUNDLED_NAMES.contains(name) {
            return Err(format!("unsupported bundled geodata: {name}"));
        }
        let meta = std::fs::symlink_metadata(source)
            .map_err(|e| format!("bundled geodata {}: {e}; run scripts/fetch-geodata.mjs before building", source.display()))?;
        let mmdb = name.ends_with(".mmdb") || name.ends_with(".metadb");
        if !is_plain_file(&meta) || meta.len() == 0
            || (mmdb && inspect_mmdb(source)? != Mmdb::Complete)
        {
            return Err(format!("invalid bundled geodata: {}", source.display()));
        }
        std::fs::File::open(source).map_err(|e| format!("read {}: {e}", source.display()))?;
    }
    Ok(())
}

/// Recover interrupted downloads before seeding. Keep valid caches (including
/// user updates); publish each bundled copy only after its write completes.
/// Service callers supply only their fixed, protected installation directory.
pub fn prepare_with_bundled(
    dir: &Path,
    sources: &[(&str, PathBuf)],
) -> Result<Preparation, String> {
    let mut preparation = prepare(dir)?;
    for (name, source) in sources {
        if !BUNDLED_NAMES.contains(name) {
            return Err(format!("unsupported bundled geodata: {name}"));
        }
        let target = dir.join(name);
        let mmdb = name.ends_with(".mmdb") || name.ends_with(".metadb");
        let ready = if mmdb {
            match inspect_mmdb(&target)? {
                Mmdb::Complete => true,
                Mmdb::Absent => false,
                Mmdb::Damaged => {
                    preparation.quarantined.push(quarantine(dir, name)?);
                    false
                }
            }
        } else {
            match std::fs::symlink_metadata(&target) {
                Ok(meta) if !is_plain_file(&meta) => {
                    return Err(format!("not a plain geodata file: {}", target.display()));
                }
                Ok(meta) if meta.len() > 0 => true,
                Ok(_) => {
                    preparation.quarantined.push(quarantine(dir, name)?);
                    false
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => return Err(e.to_string()),
            }
        };
        if ready { continue; }
        validate_bundled(&[(name, source.clone())])?;
        let temp = dir.join(format!("{name}.{}.seed", quarantine_token(0)));
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true)
            .open(&temp).map_err(|e| format!("create geodata: {e}"))?;
        let result = (|| -> std::io::Result<()> {
            let mut input = std::fs::File::open(source)?;
            std::io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            drop(output);
            std::fs::rename(&temp, &target)
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("seed {name}: {e}"));
        }
    }
    preparation.cold_start = prepare(dir)?.cold_start;
    Ok(preparation)
}

/// Readiness budget when the core has to fetch/download geodata.
pub const COLD_STARTUP: Duration = Duration::from_secs(120);
/// Readiness budget when the geodata on disk is already complete.
pub const WARM_STARTUP: Duration = Duration::from_secs(8);

/// Bytes of the file tail that are inspected (bounded read).
const MAX_TAIL: u64 = 128 * 1024;
/// MMDB metadata section marker: 0xAB 0xCD 0xEF followed by the data type name.
const MMDB_MARKER: &[u8] = b"\xAB\xCD\xEFMaxMind.com";
/// A real MMDB always has a sizeable data section before the marker; a file that
/// starts (almost) with the marker has lost its data section.
const MIN_DATA_BYTES: u64 = 16;
/// Sanity floor for the metadata that must follow the marker (real metadata is
/// ~0.5-4 KiB; this only catches a cut right at/after the marker).
const MIN_METADATA_BYTES: usize = 8;
/// How many quarantine names to try before giving up.
const QUARANTINE_ATTEMPTS: u32 = 64;

/// Outcome of `prepare`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preparation {
    /// `true` when the next core start must download or rebuild geodata and so
    /// deserves the long readiness budget.
    pub cold_start: bool,
    /// Files that were renamed aside inside the data directory (absolute paths).
    pub quarantined: Vec<PathBuf>,
}

/// Readiness budget for a core start.
pub fn startup_timeout(cold_start: bool) -> Duration {
    if cold_start {
        COLD_STARTUP
    } else {
        WARM_STARTUP
    }
}

/// Inspect `dir` and quarantine incomplete geodata. Must be called before the
/// core is spawned and while the previous core is already stopped, in the
/// application's own data directory. Returns an error instead of guessing or
/// deleting anything when the environment does not look safe.
pub fn prepare(dir: &Path) -> Result<Preparation, String> {
    ensure_real_dir(dir)?;
    let mut quarantined = Vec::new();
    let mut recovered_partial = false;
    let mut default_mmdb_ready = false;
    for name in MMDB_NAMES {
        match inspect_mmdb(&dir.join(name))? {
            Mmdb::Absent => {}
            Mmdb::Complete => {
                // A legacy Country.mmdb does not prove that mihomo's default
                // geoip.metadb is present. When unsure, keep the cold budget.
                if name == "geoip.metadb" {
                    default_mmdb_ready = true;
                }
            }
            Mmdb::Damaged => {
                // A partial MMDB makes the core re-download it: cold even if the
                // other MMDB file happens to be fine.
                recovered_partial = true;
                quarantined.push(quarantine(dir, name)?);
            }
        }
    }
    let geosite_ready = geosite_ready(dir)?;
    Ok(Preparation {
        cold_start: recovered_partial || !default_mmdb_ready || !geosite_ready,
        quarantined,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mmdb {
    Absent,
    /// Tail looks complete: marker found with data before and metadata after.
    Complete,
    /// Empty, or the MMDB tail marker/metadata is not intact -> treat as a
    /// partial download and quarantine it.
    Damaged,
}

/// The data directory itself must be a real directory, never a link/reparse
/// point. A missing directory is created (the caller owns this path); anything
/// else fails explicitly.
fn ensure_real_dir(dir: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) => {
            if is_plain_dir(&meta) {
                Ok(())
            } else {
                Err(format!(
                    "data dir is not a plain directory (link/reparse/other): {}",
                    dir.display()
                ))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(dir).map_err(|e| format!("create data dir: {e}"))?;
            match std::fs::symlink_metadata(dir) {
                Ok(meta) if is_plain_dir(&meta) => Ok(()),
                Ok(_) => Err(format!(
                    "data dir is not a plain directory: {}",
                    dir.display()
                )),
                Err(e) => Err(format!("inspect {}: {e}", dir.display())),
            }
        }
        Err(e) => Err(format!("inspect {}: {e}", dir.display())),
    }
}

fn is_plain_file(meta: &std::fs::Metadata) -> bool {
    meta.is_file() && !meta.file_type().is_symlink() && !has_reparse(meta)
}

fn is_plain_dir(meta: &std::fs::Metadata) -> bool {
    meta.is_dir() && !meta.file_type().is_symlink() && !has_reparse(meta)
}

/// Windows reparse points (symlinks, junctions, mount points) must be refused.
#[cfg(windows)]
fn has_reparse(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn has_reparse(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Classify one standard MMDB file without following links.
fn inspect_mmdb(path: &Path) -> Result<Mmdb, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Mmdb::Absent),
        Err(e) => return Err(format!("inspect {}: {e}", path.display())),
    };
    if !is_plain_file(&meta) {
        return Err(format!(
            "refusing link/reparse or non-regular geodata entry: {}",
            path.display()
        ));
    }
    if meta.len() == 0 {
        return Ok(Mmdb::Damaged);
    }
    let (tail, window_start) = read_tail(path, MAX_TAIL)?;
    if tail_looks_like_complete_mmdb(&tail, window_start) {
        Ok(Mmdb::Complete)
    } else {
        Ok(Mmdb::Damaged)
    }
}

/// Bounded read of the last `limit` bytes; also returns the file offset the
/// window starts at. Re-checks the handle's metadata so a path swapped to a link
/// between the `symlink_metadata` check and the open is rejected on length or
/// type mismatch (a residual TOCTOU window remains: the data directory is
/// expected to be protected by the service root ACL).
fn read_tail(path: &Path, limit: u64) -> Result<(Vec<u8>, u64), String> {
    use std::io::{Read, Seek, SeekFrom};
    let expected =
        std::fs::symlink_metadata(path).map_err(|e| format!("inspect {}: {e}", path.display()))?;
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let actual = file
        .metadata()
        .map_err(|e| format!("inspect {}: {e}", path.display()))?;
    if !is_plain_file(&actual) || actual.len() != expected.len() {
        return Err(format!(
            "entry changed while inspecting: {}",
            path.display()
        ));
    }
    let start = actual.len().saturating_sub(limit);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| format!("seek {}: {e}", path.display()))?;
    let mut tail = Vec::new();
    file.take(limit)
        .read_to_end(&mut tail)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok((tail, start))
}

/// Truncation sanity check, not full integrity validation: the MMDB metadata
/// marker must be present with a non-trivial data section before it and at
/// least a few metadata bytes after it.
fn tail_looks_like_complete_mmdb(tail: &[u8], window_start: u64) -> bool {
    if tail.len() <= MMDB_MARKER.len() + MIN_METADATA_BYTES {
        return false;
    }
    let last = tail.len() - MMDB_MARKER.len();
    let mut i = 0;
    while i <= last {
        if &tail[i..i + MMDB_MARKER.len()] == MMDB_MARKER {
            let after = tail.len() - (i + MMDB_MARKER.len());
            if window_start + i as u64 >= MIN_DATA_BYTES && after >= MIN_METADATA_BYTES {
                return true;
            }
            i += MMDB_MARKER.len();
        } else {
            i += 1;
        }
    }
    false
}

/// `true` when one of the standard geosite files exists as a plain, non-empty
/// file. Exact-case variants are checked by path, which is case-sensitive on
/// Unix and resolves either spelling on Windows.
fn geosite_ready(dir: &Path) -> Result<bool, String> {
    for name in GEOSITE_NAMES {
        let path = dir.join(name);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("inspect {}: {e}", path.display())),
        };
        if !is_plain_file(&meta) {
            return Err(format!(
                "refusing link/reparse or non-regular geodata entry: {}",
                path.display()
            ));
        }
        if meta.len() > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Rename a damaged file aside *within the same directory* (same volume, atomic
/// rename) under a unique `.incomplete` name. Existing files are never
/// overwritten or unlinked: if a candidate name already exists (even as a link,
/// which is not followed) another name is chosen.
fn quarantine(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let source = dir.join(name);
    for attempt in 0..QUARANTINE_ATTEMPTS {
        let candidate = dir.join(format!("{name}.{}.incomplete", quarantine_token(attempt)));
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("inspect {}: {e}", candidate.display())),
        }
        std::fs::rename(&source, &candidate)
            .map_err(|e| format!("quarantine {}: {e}", source.display()))?;
        return Ok(candidate);
    }
    Err(format!(
        "could not find a free quarantine name for {}",
        source.display()
    ))
}

/// Time + pid + attempt: unique enough inside a directory that must already be
/// protected by the service's own root ACL, and collision-checked anyway.
fn quarantine_token(attempt: u32) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}-{attempt}", std::process::id())
}

#[cfg(test)]
mod tests {
    fn bundle(dir: &Path) -> Vec<(&'static str, PathBuf)> {
        for name in BUNDLED_NAMES {
            let bytes = if name.ends_with(".mmdb") || name.ends_with(".metadb") {
                complete_mmdb()
            } else {
                b"bundled-protobuf-placeholder".to_vec()
            };
            write(&dir.join(name), &bytes);
        }
        bundled_sources(dir)
    }

    #[test]
    fn offline_first_start_and_restart_keep_manual_updates() {
        let resources = TempDir::new("bundle");
        let cache = TempDir::new("offline");
        let sources = bundle(resources.path());
        assert!(!prepare_with_bundled(cache.path(), &sources).unwrap().cold_start);
        for name in BUNDLED_NAMES {
            assert_eq!(std::fs::read(cache.path().join(name)).unwrap(), std::fs::read(resources.path().join(name)).unwrap());
        }
        let mut updated = complete_mmdb();
        updated.extend_from_slice(b"manually-updated");
        write(&cache.path().join("geoip.metadb"), &updated);
        write(&cache.path().join("geosite.dat"), b"updated-geosite");
        assert!(!prepare_with_bundled(cache.path(), &sources).unwrap().cold_start);
        assert_eq!(std::fs::read(cache.path().join("geoip.metadb")).unwrap(), updated);
        assert_eq!(std::fs::read(cache.path().join("geosite.dat")).unwrap(), b"updated-geosite");
    }

    #[test]
    fn damaged_cache_is_replaced_offline_and_preserved_as_backup() {
        let resources = TempDir::new("recovery-bundle");
        let cache = TempDir::new("recovery-cache");
        let sources = bundle(resources.path());
        // Same length as the bundle: size comparison alone cannot detect this.
        let broken = vec![0; complete_mmdb().len()];
        write(&cache.path().join("geoip.metadb"), &broken);
        write(&cache.path().join("geosite.dat"), b"");
        let result = prepare_with_bundled(cache.path(), &sources).unwrap();
        assert!(!result.cold_start);
        assert_eq!(result.quarantined.len(), 2);
        assert_eq!(std::fs::read(&result.quarantined[0]).unwrap(), broken);
    }

    #[test]
    fn invalid_bundle_fails_before_core_can_download() {
        let resources = TempDir::new("invalid-bundle");
        let cache = TempDir::new("invalid-cache");
        let sources = bundle(resources.path());
        write(&resources.path().join("geoip.metadb"), b"truncated");
        assert!(prepare_with_bundled(cache.path(), &sources).is_err());
        assert!(!cache.path().join("geoip.metadb").exists());
    }

    #[test]
    fn legacy_database_does_not_hide_missing_default_cache() {
        let tmp = TempDir::new("legacy-only");
        let mmdb = complete_mmdb();
        write(&tmp.path().join("Country.mmdb"), &mmdb);
        write(&tmp.path().join("geosite.dat"), b"geosite-bytes");
        let prep = prepare(tmp.path()).unwrap();
        assert!(prep.cold_start);
        assert!(prep.quarantined.is_empty());
        assert_eq!(std::fs::read(tmp.path().join("Country.mmdb")).unwrap(), mmdb);
    }

    #[test]
    fn warm_gui_cache_does_not_make_service_directory_warm() {
        let gui = TempDir::new("gui-cache");
        let service = TempDir::new("service-cache");
        write(&gui.path().join("geoip.metadb"), &complete_mmdb());
        write(&gui.path().join("geosite.dat"), b"geosite-bytes");
        write(&service.path().join("geosite.dat"), b"geosite-bytes");
        assert!(!prepare(gui.path()).unwrap().cold_start);
        let prep = prepare(service.path()).unwrap();
        assert!(prep.cold_start);
        assert_eq!(startup_timeout(prep.cold_start), Duration::from_secs(120));
        assert!(prep.quarantined.is_empty());
    }

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);
    /// Data section stand-in: must be >= MIN_DATA_BYTES.
    const DATA: &[u8] = b"mmdb-data-section-placeholder";
    /// Metadata stand-in: must be >= MIN_METADATA_BYTES.
    const META: &[u8] = b"metadata-block-with-extra-bytes";

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let base = std::env::var_os("PI_SCRATCH_DIR")
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
                .unwrap_or_else(std::env::temp_dir);
            for _ in 0..64 {
                let seq = SEQ.fetch_add(1, Ordering::SeqCst);
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                let path = base.join(format!(
                    "mfork-geodata-{tag}-{}-{nanos}-{seq}",
                    std::process::id()
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("create temp dir: {e}"),
                }
            }
            panic!("could not create temp dir under {}", base.display());
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn complete_mmdb() -> Vec<u8> {
        let mut bytes = DATA.to_vec();
        bytes.extend_from_slice(MMDB_MARKER);
        bytes.extend_from_slice(META);
        bytes
    }

    fn truncated_mmdb() -> Vec<u8> {
        let mut bytes = DATA.to_vec();
        bytes.extend_from_slice(MMDB_MARKER);
        bytes
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn missing_geodata_is_cold_and_touches_nothing() {
        let tmp = TempDir::new("missing");
        let prep = prepare(tmp.path()).unwrap();
        assert!(prep.cold_start);
        assert!(prep.quarantined.is_empty());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn valid_marker_starts_warm() {
        let tmp = TempDir::new("warm");
        write(&tmp.path().join("geoip.metadb"), &complete_mmdb());
        write(&tmp.path().join("geosite.dat"), b"geosite-bytes");
        let prep = prepare(tmp.path()).unwrap();
        assert!(!prep.cold_start);
        assert!(prep.quarantined.is_empty());
    }

    #[test]
    fn truncated_mmdb_is_quarantined_and_cold() {
        let tmp = TempDir::new("truncated");
        let bytes = truncated_mmdb();
        write(&tmp.path().join("geoip.metadb"), &bytes);
        write(&tmp.path().join("geosite.dat"), b"geosite-bytes");
        let prep = prepare(tmp.path()).unwrap();
        assert!(prep.cold_start);
        assert_eq!(prep.quarantined.len(), 1);
        assert!(!tmp.path().join("geoip.metadb").exists());
        let quarantined = &prep.quarantined[0];
        let name = &names(&prep.quarantined)[0];
        assert!(name.starts_with("geoip.metadb."), "{name}");
        assert!(name.ends_with(".incomplete"), "{name}");
        assert_eq!(std::fs::read(quarantined).unwrap(), bytes);
    }

    #[test]
    fn zero_length_mmdb_is_quarantined_and_cold() {
        let tmp = TempDir::new("zero");
        write(&tmp.path().join("Country.mmdb"), b"");
        write(&tmp.path().join("geosite.dat"), b"geosite-bytes");
        let prep = prepare(tmp.path()).unwrap();
        assert!(prep.cold_start);
        assert_eq!(prep.quarantined.len(), 1);
        assert!(!tmp.path().join("Country.mmdb").exists());
        assert_eq!(std::fs::read(&prep.quarantined[0]).unwrap().len(), 0);
    }

    #[test]
    fn valid_standard_files_are_preserved_untouched() {
        let tmp = TempDir::new("preserve");
        let mmdb = complete_mmdb();
        write(&tmp.path().join("geoip.metadb"), &mmdb);
        write(&tmp.path().join("Country.mmdb"), &mmdb);
        write(&tmp.path().join("geosite.dat"), b"geosite-bytes");
        let prep = prepare(tmp.path()).unwrap();
        assert!(!prep.cold_start);
        assert!(prep.quarantined.is_empty());
        assert_eq!(
            std::fs::read(tmp.path().join("geoip.metadb")).unwrap(),
            mmdb
        );
        assert_eq!(
            std::fs::read(tmp.path().join("Country.mmdb")).unwrap(),
            mmdb
        );
        assert_eq!(
            std::fs::read(tmp.path().join("geosite.dat")).unwrap(),
            b"geosite-bytes"
        );
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 3);
    }

    #[test]
    fn quarantine_is_collision_safe_and_keeps_every_backup() {
        let tmp = TempDir::new("collision");
        let first = truncated_mmdb();
        let second = b"partial-country-mmdb".to_vec();
        write(&tmp.path().join("geoip.metadb"), &first);
        write(&tmp.path().join("Country.mmdb"), &second);
        let run1 = prepare(tmp.path()).unwrap();
        assert!(run1.cold_start);
        assert_eq!(run1.quarantined.len(), 2);
        assert_ne!(run1.quarantined[0], run1.quarantined[1]);
        assert_eq!(std::fs::read(&run1.quarantined[0]).unwrap(), first);
        assert_eq!(std::fs::read(&run1.quarantined[1]).unwrap(), second);

        write(&tmp.path().join("geoip.metadb"), &first);
        let run2 = prepare(tmp.path()).unwrap();
        assert_eq!(run2.quarantined.len(), 1);
        let mut all = names(&run1.quarantined);
        all.extend(names(&run2.quarantined));
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 3, "quarantine names must not collide: {all:?}");
        for path in run1.quarantined.iter().chain(run2.quarantined.iter()) {
            assert!(path.is_file());
        }
        assert_eq!(std::fs::read(&run2.quarantined[0]).unwrap(), first);
    }

    #[test]
    fn missing_geosite_is_cold_but_keeps_valid_mmdb() {
        let tmp = TempDir::new("nogeosite");
        let mmdb = complete_mmdb();
        write(&tmp.path().join("geoip.metadb"), &mmdb);
        let prep = prepare(tmp.path()).unwrap();
        assert!(prep.cold_start);
        assert!(prep.quarantined.is_empty());
        assert_eq!(
            std::fs::read(tmp.path().join("geoip.metadb")).unwrap(),
            mmdb
        );
    }

    #[test]
    fn empty_geosite_is_cold_and_keeps_valid_mmdb() {
        let tmp = TempDir::new("emptygeosite");
        let mmdb = complete_mmdb();
        write(&tmp.path().join("geoip.metadb"), &mmdb);
        write(&tmp.path().join("geosite.dat"), b"");
        let prep = prepare(tmp.path()).unwrap();
        assert!(prep.cold_start);
        assert!(prep.quarantined.is_empty());
        assert_eq!(
            std::fs::read(tmp.path().join("geoip.metadb")).unwrap(),
            mmdb
        );
        assert_eq!(
            std::fs::read(tmp.path().join("geosite.dat")).unwrap().len(),
            0
        );
    }

    /// `GeoSite.dat` is only a distinct file on a case-sensitive filesystem.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn legacy_geosite_case_is_accepted() {
        let tmp = TempDir::new("geositecase");
        write(&tmp.path().join("geoip.metadb"), &complete_mmdb());
        write(&tmp.path().join("GeoSite.dat"), b"legacy-geosite");
        let prep = prepare(tmp.path()).unwrap();
        assert!(!prep.cold_start);
        assert!(prep.quarantined.is_empty());
    }

    #[test]
    fn warm_timeout_is_eight_seconds_and_cold_is_two_minutes() {
        assert_eq!(startup_timeout(false), Duration::from_secs(8));
        assert_eq!(startup_timeout(true), Duration::from_secs(120));
        assert_eq!(WARM_STARTUP, Duration::from_secs(8));
        assert_eq!(COLD_STARTUP, Duration::from_secs(120));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_mmdb_is_rejected_and_target_is_kept() {
        let tmp = TempDir::new("link");
        let target = tmp.path().join("real.metadb");
        write(&target, &complete_mmdb());
        std::os::unix::fs::symlink(&target, tmp.path().join("geoip.metadb")).unwrap();
        assert!(prepare(tmp.path()).is_err());
        assert!(std::fs::symlink_metadata(tmp.path().join("geoip.metadb"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), complete_mmdb());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_data_directory_is_rejected() {
        let tmp = TempDir::new("dirlink");
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        write(&real.join("geoip.metadb"), &complete_mmdb());
        let link = tmp.path().join("data");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(prepare(&link).is_err());
        assert!(real.join("geoip.metadb").is_file());
    }

    #[test]
    fn non_regular_entries_are_rejected_instead_of_deleted() {
        let tmp = TempDir::new("nondir");
        std::fs::create_dir(tmp.path().join("geoip.metadb")).unwrap();
        write(&tmp.path().join("geoip.metadb").join("sentinel"), b"keep");
        assert!(prepare(tmp.path()).is_err());
        assert_eq!(
            std::fs::read(tmp.path().join("geoip.metadb").join("sentinel")).unwrap(),
            b"keep"
        );
    }
}
