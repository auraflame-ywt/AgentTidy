//! Cross-process cleanup operation guard (`Start.md` §12.1 "Acquire
//! operation guard").
//!
//! Only one cleanup execution may run per user at a time: a second GUI
//! window or CLI instance executing the same plan concurrently would
//! race on the same transcript files and produce duplicate audit rows
//! (§12.3 requires "already moved ⇒ not a fresh success").
//!
//! Mechanism: an atomic `create_new` lock file at
//! `$HOME/.agenttidy/operations.lock` (same state dir as the §16.3 audit
//! log). `O_EXCL` create is atomic on every supported platform, so no
//! platform `#[cfg]` is needed here (red line #11 stays intact).
//!
//! Staleness (§12.3 crash recovery): a crashed run leaves the lock file
//! behind. The lock records the holder's pid + start timestamp for
//! diagnostics, but liveness is judged by **file age** — a lock older
//! than [`STALE_AFTER_MS`] is considered abandoned and is taken over.
//! Age-based (not pid-based) because Windows has no std pid-liveness
//! API and the cleanup batch cannot outlive the 5-minute plan TTL by
//! design, so the 10-minute threshold cannot mask a live holder.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Lock older than this is assumed abandoned. Deliberately larger than
/// the 5-minute `CleanupPlan` TTL: a live execution always finishes
/// (or fails) within one plan lifetime.
pub const STALE_AFTER_MS: u128 = 10 * 60 * 1000;

const LOCK_FILE: &str = "operations.lock";

/// Failure to acquire the operation guard. `HeldByOtherProcess` is the
/// expected contention case — the executor surfaces it verbatim and
/// touches nothing (fail-closed).
#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error("home directory is not set; cannot resolve operation lock path")]
    NoHome,
    #[error("another cleanup operation is in progress (lock {path}: {holder})", path = .path.display())]
    HeldByOtherProcess { path: PathBuf, holder: String },
    #[error("failed to access operation lock {path}: {source}", path = .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// RAII guard: holds the lock file until dropped, then removes it.
/// Removal errors on drop are ignored (the stale-takeover path recovers
/// from a lock file that outlives its process).
#[derive(Debug)]
pub struct OperationGuard {
    lock_path: PathBuf,
}

impl OperationGuard {
    /// Acquire the per-user lock at the default location.
    pub fn acquire() -> Result<Self, GuardError> {
        let home = crate::paths::home_dir().ok_or(GuardError::NoHome)?;
        Self::acquire_at(&home.join(crate::audit_log::AUDIT_LOG_DIR).join(LOCK_FILE))
    }

    /// Acquire the lock at an explicit path (tests use temp paths so
    /// they never contend with the real per-user lock).
    pub fn acquire_at(lock_path: &Path) -> Result<Self, GuardError> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| GuardError::Io {
                path: lock_path.to_path_buf(),
                source,
            })?;
        }
        match Self::try_create(lock_path) {
            Ok(()) => Ok(Self {
                lock_path: lock_path.to_path_buf(),
            }),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                // One stale-takeover retry: remove the abandoned lock and
                // race for a fresh create. `create_new` keeps the race
                // single-winner; a second AlreadyExists means a live (or
                // just-created) holder and we fail closed.
                if !lock_is_stale(lock_path) {
                    return Err(GuardError::HeldByOtherProcess {
                        path: lock_path.to_path_buf(),
                        holder: read_holder(lock_path),
                    });
                }
                std::fs::remove_file(lock_path).map_err(|source| GuardError::Io {
                    path: lock_path.to_path_buf(),
                    source,
                })?;
                Self::try_create(lock_path)
                    .map(|_| Self {
                        lock_path: lock_path.to_path_buf(),
                    })
                    .map_err(|source| {
                        if source.kind() == io::ErrorKind::AlreadyExists {
                            GuardError::HeldByOtherProcess {
                                path: lock_path.to_path_buf(),
                                holder: read_holder(lock_path),
                            }
                        } else {
                            GuardError::Io {
                                path: lock_path.to_path_buf(),
                                source,
                            }
                        }
                    })
            }
            Err(source) => Err(GuardError::Io {
                path: lock_path.to_path_buf(),
                source,
            }),
        }
    }

    /// Create the lock file atomically and stamp it with pid + start
    /// time (diagnostics only — see the module doc on why liveness is
    /// age-based).
    fn try_create(lock_path: &Path) -> io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        io::Write::write_all(
            &mut file,
            format!("pid={}\nstarted_ms={stamp}\n", std::process::id()).as_bytes(),
        )?;
        Ok(())
    }

    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        std::fs::remove_file(&self.lock_path).ok();
    }
}

/// A lock is stale when its file mtime is older than
/// [`STALE_AFTER_MS`] (or unreadable — fail toward takeover, since an
/// unreadable stamp cannot prove a live holder and the create race is
/// still single-winner).
fn lock_is_stale(lock_path: &Path) -> bool {
    let Some(modified_ms) = std::fs::symlink_metadata(lock_path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|stamp| stamp.duration_since(UNIX_EPOCH).ok())
        .map(|since_epoch| since_epoch.as_millis())
    else {
        return true;
    };
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|now| now.as_millis())
        .unwrap_or(modified_ms);
    now_ms.saturating_sub(modified_ms) > STALE_AFTER_MS
}

/// Best-effort holder description (pid + start stamp) for the
/// contention error message.
fn read_holder(lock_path: &Path) -> String {
    std::fs::read_to_string(lock_path)
        .ok()
        .map(|contents| contents.trim().replace('\n', ", "))
        .filter(|contents| !contents.is_empty())
        .unwrap_or_else(|| "holder unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_lock_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agenttidy-guard-{}-{}-{tag}.lock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn guard_acquires_and_releases_on_drop() {
        let path = temp_lock_path("acquire");
        {
            let guard = OperationGuard::acquire_at(&path).expect("first acquire succeeds");
            assert!(path.exists(), "lock file exists while held");
            assert!(guard.lock_path() == path.as_path());
        }
        assert!(!path.exists(), "lock file removed on drop");
    }

    #[test]
    fn second_acquire_while_held_fails_closed() {
        let path = temp_lock_path("contended");
        let _guard = OperationGuard::acquire_at(&path).expect("first acquire succeeds");
        let error = OperationGuard::acquire_at(&path)
            .expect_err("second acquire must fail while the lock is held");
        assert!(matches!(error, GuardError::HeldByOtherProcess { .. }));
        // Fail-closed also means the holder is identified for diagnostics.
        assert!(error.to_string().contains("another cleanup operation"));
    }

    #[test]
    fn stale_lock_is_taken_over() {
        let path = temp_lock_path("stale");
        let guard = OperationGuard::acquire_at(&path).expect("first acquire succeeds");
        // Simulate a crashed run: keep the lock file but backdate its
        // mtime past the staleness threshold, then leak the guard so the
        // file survives (std::mem::forget skips Drop).
        let stale_stamp =
            SystemTime::now() - std::time::Duration::from_millis(STALE_AFTER_MS as u64 + 60_000);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open held lock");
        file.set_modified(stale_stamp).expect("backdate lock mtime");
        std::mem::forget(guard);

        let taken_over = OperationGuard::acquire_at(&path).expect("stale lock is taken over");
        drop(taken_over);
        assert!(!path.exists());
    }

    #[test]
    fn fresh_lock_is_not_taken_over() {
        let path = temp_lock_path("fresh");
        let guard = OperationGuard::acquire_at(&path).expect("first acquire succeeds");
        // Not stale — even an unparseable/odd stamp must not unblock a
        // second executor while the lock is young.
        assert!(matches!(
            OperationGuard::acquire_at(&path),
            Err(GuardError::HeldByOtherProcess { .. })
        ));
        drop(guard);
        assert!(!path.exists());
    }
}
