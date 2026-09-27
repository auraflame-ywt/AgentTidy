//! Local append-only audit log for cleanup operations (`Start.md` §16.3).
//!
//! One JSONL file per user, written via `fsync` so a crash mid-line
//! produces a truncated trailing record that the JSONL reader will skip
//! (the philosophy at `crates/infrastructure/src/jsonl.rs:142-145`).
//!
//! The log never contains transcript bodies, credentials, or file
//! contents — only paths and metadata (`Start.md` §16.3 line 944).
//!
//! Rotation policy: **none in v0.1** — the user is responsible for
//! truncating `$HOME/.agenttidy/operations.jsonl` when it grows. The
//! exact wording here is deliberately conservative; a future rotation
//! hook will record-but-not-truncate.
//!
//! Why this lives in infrastructure: the file path comes from `paths::home_dir`
//! (which itself isolates the macOS/Windows env var lookup per red line
//! #11), and `fsync` is an OS-mac detail.

use agenttidy_core::CleanupEvent;
use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Shared per-user state directory (`.agenttidy`), also home to the
/// §12.1 operation lock — `pub(crate)` so both file-adjacent concerns
/// agree on one location without a second source of truth.
pub(crate) const AUDIT_LOG_DIR: &str = ".agenttidy";
const AUDIT_LOG_FILE: &str = "operations.jsonl";

/// Audit log open / write errors. The executor maps these to
/// `CleanupOutcome::Failed`; the log failure never blocks a successful
/// trash operation, but it *does* abort the rest of the batch so the
/// user sees a uniform failure (no half-recorded plans).
#[derive(Debug, Error)]
pub enum AuditLogError {
    #[error("home directory is not set; cannot resolve audit log path")]
    NoHome,
    #[error("failed to access audit log directory {}: {source}", path.display())]
    Directory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize cleanup event: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write audit log line: {source}")]
    Write {
        #[source]
        source: std::io::Error,
    },
}

/// Path to the per-user audit log file. Created lazily by [`record`].
///
/// `$HOME/.agenttidy/operations.jsonl` on macOS / Linux,
/// `%USERPROFILE%\.agenttidy\operations.jsonl` on Windows.
pub fn audit_log_path() -> Result<PathBuf, AuditLogError> {
    let home = crate::paths::home_dir().ok_or(AuditLogError::NoHome)?;
    Ok(home.join(AUDIT_LOG_DIR).join(AUDIT_LOG_FILE))
}

/// Append one [`CleanupEvent`] to the log.
///
/// The full file is opened in append mode + `fsync` per write — slow but
/// crash-resilient. Phase 6's executor does not run cleanup at high
/// frequency, so the cost is acceptable; Phase 7 can introduce a
/// buffered writer if needed.
pub fn record(event: &CleanupEvent) -> Result<(), AuditLogError> {
    let path = audit_log_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| AuditLogError::Directory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut line = serde_json::to_string(event).map_err(AuditLogError::Serialize)?;
    line.push('\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|source| AuditLogError::Write { source })?;
    file.write_all(line.as_bytes())
        .map_err(|source| AuditLogError::Write { source })?;
    file.sync_all()
        .map_err(|source| AuditLogError::Write { source })?;
    Ok(())
}

/// Read all recorded events (test/inspection helper). Not used by the
/// executor — the log is append-only and audit consumers should rely on
/// their own streaming reader.
pub fn read_all(path: &Path) -> Result<Vec<CleanupEvent>, AuditLogError> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).map_err(|source| AuditLogError::Write { source })?;
    let mut events = Vec::new();
    for (index, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|source| AuditLogError::Write { source })?;
        // Trailing partial lines after a crash are intentionally skipped
        // (matches jsonl.rs:142-145) — but a clean mid-file parse error
        // is surfaced so consumers know the log is corrupt.
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<CleanupEvent>(&line) {
            Ok(event) => events.push(event),
            Err(error) => {
                eprintln!(
                    "audit_log: skipping malformed line {} ({error:?})",
                    index + 1
                );
            }
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agenttidy_core::{
        CleanupAction, CleanupEvent, CleanupLocator, CleanupOutcome, Platform as CorePlatform,
        ProviderId,
    };
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// HOME mutation is process-wide; serialise the audit_log tests so
    /// the env var never changes under another test mid-run.
    static HOME_LOCK: Mutex<()> = Mutex::new(());

    fn temp_home(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agenttidy-audit-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct HomeGuard {
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    /// The env var `paths::home_dir` actually reads on this platform
    /// (`USERPROFILE` on Windows, `HOME` elsewhere — see paths.rs).
    /// Setting only `HOME` would leave `home_dir()` pinned to the real
    /// user profile on Windows and the assertions would target the
    /// wrong tree.
    #[cfg(windows)]
    const HOME_VAR: &str = "USERPROFILE";
    #[cfg(not(windows))]
    const HOME_VAR: &str = "HOME";

    fn set_home(path: &Path) -> HomeGuard {
        let lock = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let previous = std::env::var_os(HOME_VAR);
        // SAFETY: serialised by HOME_LOCK.
        unsafe {
            std::env::set_var(HOME_VAR, path.as_os_str());
        }
        HomeGuard {
            previous,
            _lock: lock,
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            // SAFETY: serialised by HOME_LOCK (still held via _lock).
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(HOME_VAR, value),
                    None => std::env::remove_var(HOME_VAR),
                }
            }
        }
    }

    fn event(timestamp_ms: u64, outcome: CleanupOutcome) -> CleanupEvent {
        CleanupEvent {
            schema_version: CleanupEvent::SCHEMA_VERSION.into(),
            platform: CorePlatform::Macos,
            timestamp_ms,
            plan_id: "plan-test".into(),
            unit_id: "unit-test".into(),
            provider: ProviderId::new(ProviderId::CODEX),
            original_locator: CleanupLocator::Dir {
                path: PathBuf::from("/x"),
            },
            action: CleanupAction::Trash,
            outcome,
            reclaimed_bytes: 42,
            reason: None,
        }
    }

    #[test]
    fn audit_log_path_resolves_under_home_dot_agenttidy() {
        let home = temp_home("path");
        let _guard = set_home(&home);
        let resolved = audit_log_path().unwrap();
        assert_eq!(resolved, home.join(".agenttidy").join("operations.jsonl"));
    }

    #[test]
    fn append_only_round_trips_with_fsync() {
        let home = temp_home("append");
        let path = home.join(".agenttidy").join("operations.jsonl");
        let _guard = set_home(&home);

        record(&event(1_700_000_000_000, CleanupOutcome::Removed)).unwrap();
        record(&event(1_700_000_000_500, CleanupOutcome::Skipped)).unwrap();
        record(&event(1_700_000_001_000, CleanupOutcome::Failed)).unwrap();

        let events = read_all(&path).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].outcome, CleanupOutcome::Removed);
        assert_eq!(events[1].outcome, CleanupOutcome::Skipped);
        assert_eq!(events[2].outcome, CleanupOutcome::Failed);
        assert!(events
            .iter()
            .all(|e| e.schema_version == CleanupEvent::SCHEMA_VERSION));

        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn record_creates_dot_agenttidy_dir_lazily() {
        let home = temp_home("lazy");
        let _guard = set_home(&home);
        // The directory is removed so the audit log must recreate it.
        assert!(!home.join(".agenttidy").exists());
        record(&event(1, CleanupOutcome::Removed)).unwrap();
        assert!(home.join(".agenttidy").join("operations.jsonl").exists());
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn audit_log_path_returns_no_home_when_home_unset() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let previous = std::env::var_os(HOME_VAR);
        // SAFETY: serialised by HOME_LOCK.
        unsafe {
            std::env::remove_var(HOME_VAR);
        }
        match audit_log_path() {
            Err(AuditLogError::NoHome) => {}
            other => panic!("expected NoHome, got {other:?}"),
        }
        if let Some(value) = previous {
            // SAFETY: serialised by HOME_LOCK.
            unsafe {
                std::env::set_var(HOME_VAR, value);
            }
        }
    }
}
