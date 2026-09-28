//! Bounded startup diagnostics, also usable after a failed core has been stopped.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_TAIL_BYTES: u64 = 64 * 1024;
const MAX_TAIL_LINES: usize = 2000;

pub fn tail(dir: &Path, lines: usize) -> String {
    let lines = lines.min(MAX_TAIL_LINES);
    if lines == 0 {
        return String::new();
    }
    let mut out = String::new();
    for name in ["core.err.log", "core.log"] {
        match file_tail(&dir.join(name), lines) {
            Ok(text) if !text.is_empty() => {
                out.push_str(&format!("── {name}\n{text}"));
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                out.push_str(&format!("── {name}: unable to read log: {e}\n"));
            }
            _ => {}
        }
    }
    out
}

fn file_tail(path: &Path, lines: usize) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let offset = file.metadata()?.len().saturating_sub(MAX_TAIL_BYTES);
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(MAX_TAIL_BYTES).read_to_end(&mut bytes)?;
    // Do not show an arbitrary half-line (or split UTF-8 sequence) at the boundary.
    let bytes = if offset > 0 {
        match bytes.iter().position(|b| *b == b'\n') {
            Some(i) => &bytes[i + 1..],
            None => &[],
        }
    } else {
        &bytes[..]
    };
    let text = String::from_utf8_lossy(bytes);
    let selected: Vec<_> = text.lines().rev().take(lines).collect();
    let mut out = String::new();
    for line in selected.iter().rev() {
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

pub fn startup_error(pid: u32, reason: &str, dir: &Path) -> String {
    let mut message = format!(
        "mihomo (pid {pid}) {reason} — see core.log / core.err.log in {}",
        dir.display()
    );
    let logs = tail(dir, 12);
    if !logs.is_empty() {
        message.push('\n');
        message.push_str(&logs);
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let root = std::env::var_os("PI_SCRATCH_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let path = root.join(format!(
                "mfork-log-test-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reads_both_logs_in_order_and_limits_lines() {
        let dir = TestDir::new();
        std::fs::write(dir.0.join("core.err.log"), "older\nerror\n").unwrap();
        std::fs::write(dir.0.join("core.log"), "older\ndownloading\n").unwrap();
        assert_eq!(tail(&dir.0, 1), "── core.err.log\nerror\n── core.log\ndownloading\n");
        assert!(tail(&dir.0, 0).is_empty());
    }

    #[test]
    fn failure_uses_actual_service_log_directory() {
        let gui = TestDir::new();
        let service = TestDir::new();
        std::fs::write(gui.0.join("core.log"), "unrelated GUI log").unwrap();
        std::fs::write(service.0.join("core.log"), "Can't find MMDB, start download").unwrap();
        let message = startup_error(42, "did not open the API port within 120s", &service.0);
        assert!(message.contains("Can't find MMDB"));
        assert!(message.contains(&service.0.display().to_string()));
        assert!(message.contains("core.log / core.err.log"));
        assert!(!message.contains("unrelated GUI log"));
    }

    #[test]
    fn large_files_are_bounded_and_partial_first_line_is_dropped() {
        let dir = TestDir::new();
        let mut text = "x".repeat(MAX_TAIL_BYTES as usize + 100);
        text.push_str("\n完整尾行\n");
        std::fs::write(dir.0.join("core.log"), text).unwrap();
        assert_eq!(tail(&dir.0, usize::MAX), "── core.log\n完整尾行\n");
    }

    #[test]
    fn missing_logs_still_report_the_correct_paths() {
        let dir = TestDir::new();
        assert!(tail(&dir.0, 20).is_empty());
        let message = startup_error(42, "exited before opening the API port", &dir.0);
        assert!(message.contains(&dir.0.display().to_string()));
        assert!(message.contains("core.log / core.err.log"));
    }
}
