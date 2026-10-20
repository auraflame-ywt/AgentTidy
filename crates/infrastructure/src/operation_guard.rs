//! Cross-process execution guard (`Start.md` §12.1 / §12.3).
//!
//! An OS advisory lock is held on a persistent per-user lock file. The OS
//! releases it when the process exits, including crashes; file age never
//! authorizes takeover of a live operation. The file must not be unlinked:
//! doing so would let a second process lock a different inode at the same path.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const LOCK_FILE: &str = "operations.lock";
const PROTOCOL: &str = "lock_protocol=2\n";

#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error("home directory is not set; cannot resolve operation lock path")]
    NoHome,
    #[error("another cleanup operation may be in progress (lock {path}: {holder})", path = .path.display())]
    HeldByOtherProcess { path: PathBuf, holder: String },
    #[error("failed to access operation lock {path}: {source}", path = .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Closing the held file releases the OS lock. The diagnostic file remains
/// so every future process opens and locks the same inode (§12.1).
#[derive(Debug)]
pub struct OperationGuard {
    lock_path: PathBuf,
    _file: File,
}

impl OperationGuard {
    pub fn acquire() -> Result<Self, GuardError> {
        let home = crate::paths::home_dir().ok_or(GuardError::NoHome)?;
        Self::acquire_at(&home.join(crate::audit_log::AUDIT_LOG_DIR).join(LOCK_FILE))
    }

    /// Tests and native acceptance use explicit temporary paths, never the
    /// user's live operation lock. Unknown legacy lock files fail closed.
    pub fn acquire_at(lock_path: &Path) -> Result<Self, GuardError> {
        let io_error = |source| GuardError::Io {
            path: lock_path.to_path_buf(),
            source,
        };
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(io_error)?;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(lock_path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(io_error(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "operation lock must be a regular file, not a link",
                )));
            }
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .map_err(io_error)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(GuardError::HeldByOtherProcess {
                    path: lock_path.to_path_buf(),
                    holder: "OS lock held by another process".into(),
                })
            }
            Err(TryLockError::Error(error)) => return Err(io_error(error)),
        }
        let mut previous = String::new();
        Read::by_ref(&mut file)
            .take(4096)
            .read_to_string(&mut previous)
            .map_err(io_error)?;
        if !previous.is_empty() && !previous.starts_with(PROTOCOL) {
            // Older clients do not hold an OS lock. Never assume their pid
            // stamp is abandoned just because the file is old (§16.2).
            return Err(GuardError::HeldByOtherProcess {
                path: lock_path.to_path_buf(),
                holder: "legacy or unknown lock; close older clients and inspect it before removal"
                    .into(),
            });
        }
        file.set_len(0).map_err(io_error)?;
        file.rewind().map_err(io_error)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        write!(
            file,
            "{PROTOCOL}pid={}\nstarted_ms={timestamp}\n",
            std::process::id()
        )
        .map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        Ok(Self {
            lock_path: lock_path.to_path_buf(),
            _file: file,
        })
    }

    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    fn temporary_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agenttidy-guard-{}-{}-{label}.lock",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn guard_releases_without_unlinking_the_shared_inode() {
        let path = temporary_path("release");
        let guard = OperationGuard::acquire_at(&path).unwrap();
        assert_eq!(guard.lock_path(), path);
        assert!(matches!(
            OperationGuard::acquire_at(&path),
            Err(GuardError::HeldByOtherProcess { .. })
        ));
        drop(guard);
        assert!(path.exists());
        drop(OperationGuard::acquire_at(&path).unwrap());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn old_mtime_never_steals_a_live_lock() {
        let path = temporary_path("old-live");
        let guard = OperationGuard::acquire_at(&path).unwrap();
        let stamp = SystemTime::now() - Duration::from_secs(24 * 60 * 60);
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        assert!(matches!(
            OperationGuard::acquire_at(&path),
            Err(GuardError::HeldByOtherProcess { .. })
        ));
        drop(guard);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn unknown_legacy_stamp_is_never_automatically_deleted() {
        let path = temporary_path("legacy");
        std::fs::write(&path, "pid=123\nstarted_ms=0\n").unwrap();
        assert!(matches!(
            OperationGuard::acquire_at(&path),
            Err(GuardError::HeldByOtherProcess { .. })
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "pid=123\nstarted_ms=0\n"
        );
        std::fs::remove_file(path).unwrap();
    }

    /// Child-only harness. Command env settings are isolated to the child;
    /// no process-global HOME or lock-path mutations race other tests.
    #[test]
    fn child_holds_lock_until_terminated() {
        let Some(path) = std::env::var_os("AGENTTIDY_CHILD_LOCK") else {
            return;
        };
        let path = PathBuf::from(path);
        let _guard = OperationGuard::acquire_at(&path).unwrap();
        std::fs::write(path.with_extension("ready"), b"ready").unwrap();
        // A finite lifetime also prevents an orphan if its parent panics.
        std::thread::sleep(Duration::from_secs(10));
    }

    #[test]
    fn process_crash_releases_the_native_lock() {
        let path = temporary_path("crash");
        let ready = path.with_extension("ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "operation_guard::tests::child_holds_lock_until_terminated",
            ])
            .env("AGENTTIDY_CHILD_LOCK", &path)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if ready.exists() {
                break;
            }
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let started = ready.exists();
        let blocked = started
            && matches!(
                OperationGuard::acquire_at(&path),
                Err(GuardError::HeldByOtherProcess { .. })
            );
        child.kill().ok();
        child.wait().unwrap();
        assert!(
            started && blocked,
            "child must acquire and exclusively hold the lock"
        );
        drop(OperationGuard::acquire_at(&path).unwrap());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(ready).unwrap();
    }
}
