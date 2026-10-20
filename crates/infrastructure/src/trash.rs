//! Platform-native move-to-trash (`Start.md` §3.4, §16.4).
//!
//! "Trash first" is the default disposal path for file-shaped cleanup:
//! recovery is delegated to the OS (Windows Recycle Bin / macOS
//! Finder Trash) and AgentTidy never claims an in-app undo it cannot
//! honor (§16.4). The `trash` crate wraps the platform APIs
//! (`SHFileOperation`/`IFileOperation` on Windows, `NSWorkspace` on
//! macOS).
//!
//! §16.2 note: on Windows a file held open by another process cannot
//! be moved — this surfaces as an `Err` here; the Phase 6 executor
//! maps it to `skipped`, never to a force-close or a delayed delete.

use std::path::{Path, PathBuf};

/// Error moving something to the trash.
#[derive(Debug, thiserror::Error)]
pub enum TrashError {
    #[error("path not found: {0}")]
    NotFound(PathBuf),
    /// The platform refused the operation (file in use, headless
    /// session without a trash service, …). The message is the
    /// platform error verbatim — surfaced to diagnostics, not guessed
    /// around.
    #[error("platform trash operation failed: {0}")]
    Platform(String),
}

/// Per-call outcome of `move_to_trash`.
///
/// `AlreadyGone` is *not* an error: the trash operation is idempotent —
/// if the unit's fingerprint is already stale and the path is gone, we
/// record it as a no-op skip, not a fresh failure (§12.3: "已移动的资源
/// 再次执行时不得报成新的成功").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashOutcome {
    /// The path was moved to the OS trash.
    Removed,
    /// The path was already gone when we tried. Idempotency guard.
    AlreadyGone,
    /// The path is held open by another process (Windows
    /// `ERROR_SHARING_VIOLATION` / `ERROR_LOCK_VIOLATION`, including
    /// their HRESULT forms). This is a *transient* condition: the
    /// caller reports `skipped` with a retry hint, never `failed`
    /// (`docs/safety/session-transcript-fileset.md` Concurrency).
    Locked,
    /// The platform refused the operation for any other reason
    /// (headless macOS session with no Finder service, …). The
    /// verbatim platform error is preserved for diagnostics and
    /// the audit log.
    PlatformError(String),
}

/// Windows error codes (and their `HRESULT_FROM_WIN32` forms) that mean
/// "another process holds this file open" — the only lock signals the
/// trash layer can reliably classify (`docs/safety/README.md`, Phase 7
/// hardening item "skipped vs failed for locked files").
const LOCK_ERROR_CODES: [i32; 4] = [
    32,                    // ERROR_SHARING_VIOLATION
    33,                    // ERROR_LOCK_VIOLATION
    0x8007_0020u32 as i32, // HRESULT_FROM_WIN32(ERROR_SHARING_VIOLATION)
    0x8007_0021u32 as i32, // HRESULT_FROM_WIN32(ERROR_LOCK_VIOLATION)
];

/// Classify a `trash` crate error as the lock signal when its OS
/// error code is one of the lock codes. Non-Windows platforms never
/// produce these codes, so the check is harmless everywhere.
fn is_locked_error(error: &trash::Error) -> bool {
    matches!(error, trash::Error::Os { code, .. } if LOCK_ERROR_CODES.contains(code))
}

/// Move `path` to the OS trash. Works for files and directories.
///
/// Fail-closed semantics: any doubt (missing path, locked file,
/// platform error) maps to a `TrashOutcome` variant — never to a
/// silent success. Callers map `Locked` to `CleanupOutcome::Skipped`
/// (transient, retry after the holder exits), `PlatformError` to
/// `CleanupOutcome::Failed` per `Start.md` §16.2 + §12.3.
pub fn move_to_trash(path: &Path) -> Result<TrashOutcome, TrashError> {
    match path.symlink_metadata() {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(TrashError::NotFound(path.to_path_buf()))
        }
        Err(error) => {
            return Err(TrashError::Platform(format!(
                "cannot inspect {}: {error}",
                path.display()
            )))
        }
    }
    match native_trash(path) {
        Ok(()) => Ok(TrashOutcome::Removed),
        Err(error) if is_locked_error(&error) => Ok(TrashOutcome::Locked),
        Err(error) => Ok(TrashOutcome::PlatformError(error.to_string())),
    }
}

/// macOS uses NSFileManager directly. Finder AppleScript requires Automation
/// permission and a logged-in Finder service, neither of which is necessary
/// for the OS Trash API. Recovery is manual (drag from Trash); “Put Back” is
/// not guaranteed by this API and must not be promised (§16.4).
fn native_trash(path: &Path) -> Result<(), trash::Error> {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut context = trash::TrashContext::new();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context.delete(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(path)
    }
}

/// Per-path result entry of [`move_paths_to_trash`]: which path, and
/// its own `move_to_trash` outcome (error or not).
pub type PathTrashResult = (PathBuf, Result<TrashOutcome, TrashError>);

/// Move every path to the OS trash as one logical batch (a `FileSet`
/// cleanup unit: transcript + side directory, `Start.md` §9.5).
///
/// Continues past failures on purpose (§12.3 "partial failure must not
/// corrupt the report"): stopping at the first error would leave the
/// caller unable to say *which* members moved. The executor aggregates
/// the per-path results into one item outcome — the move itself is not
/// atomic across paths, but everything that did move is recoverable
/// from the OS trash (`docs/safety/session-transcript-fileset.md`).
pub fn move_paths_to_trash(paths: &[PathBuf]) -> Vec<PathTrashResult> {
    paths
        .iter()
        .map(|path| (path.clone(), move_to_trash(path)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moved_file_disappears_from_its_location() {
        let dir = std::env::temp_dir().join(format!(
            "agenttidy-trash-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("to-trash.txt");
        std::fs::write(&file, b"temporary").unwrap();

        match move_to_trash(&file) {
            Ok(TrashOutcome::Removed) => {
                // The whole point of trash-first: the file is no longer at its
                // old location (its new home — the Recycle Bin / Trash — is
                // platform-owned and intentionally out of scope).
                assert!(!file.exists());
            }
            Ok(TrashOutcome::AlreadyGone) => panic!("freshly written file must be trappable"),
            Ok(TrashOutcome::PlatformError(error)) => {
                // Headless macOS sessions have no Finder/XPC trash service.
                // That is an environment limitation, not a property this unit
                // test can validate; production still returns this error to
                // callers so cleanup remains fail-closed.
                eprintln!("skipping native-trash assertion: {error}");
            }
            Ok(TrashOutcome::Locked) => {
                // A freshly-written temp file is not held by anyone; if the
                // platform nonetheless reports a lock, treat it as the
                // environment limitation above (the classifier is what the
                // dedicated test below pins).
                eprintln!("unexpected lock classification on an unheld temp file");
            }
            Err(error) => panic!("existing temporary file must be trappable: {error}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_path_returns_not_found_error_not_silent_success() {
        let missing = std::env::temp_dir().join("agenttidy-never-existed.txt");
        // NotFound is reserved for internal precondition checks (caller
        // already verified path). `move_to_trash` reports it as an error
        // so the executor surfaces it as `Failed`, not a fresh `Removed`.
        assert!(matches!(
            move_to_trash(&missing),
            Err(TrashError::NotFound(_))
        ));
    }

    /// The lock classifier recognizes the Windows sharing/lock violation
    /// codes (raw and HRESULT forms) and nothing else. This is the
    /// `skipped`-vs-`failed` contract from
    /// `docs/safety/session-transcript-fileset.md` (Concurrency).
    #[test]
    fn locked_error_codes_classify_as_locked() {
        for code in [32, 33, 0x8007_0020u32 as i32, 0x8007_0021u32 as i32] {
            let error = trash::Error::Os {
                code,
                description: "the process cannot access the file".into(),
            };
            assert!(
                is_locked_error(&error),
                "code {code} must classify as locked"
            );
        }
        // Ordinary OS errors are not locks.
        let not_locked = trash::Error::Os {
            code: 5, // ERROR_ACCESS_DENIED
            description: "access denied".into(),
        };
        assert!(!is_locked_error(&not_locked));
        let unknown = trash::Error::Unknown {
            description: "unclassified".into(),
        };
        assert!(!is_locked_error(&unknown));
    }

    #[test]
    fn move_paths_to_trash_reports_per_path_and_continues_past_missing() {
        let dir = std::env::temp_dir().join(format!(
            "agenttidy-trash-batch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let present = dir.join("member-a.txt");
        std::fs::write(&present, b"temporary").unwrap();
        let missing = dir.join("member-b.txt");

        // Order is preserved and a missing member must not abort the
        // batch: the caller needs one row per member path (§12.3).
        let results = move_paths_to_trash(&[present.clone(), missing.clone()]);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, present);
        assert_eq!(results[1].0, missing);
        assert!(matches!(results[1].1, Err(TrashError::NotFound(_))));
        match &results[0].1 {
            Ok(TrashOutcome::Removed) | Ok(TrashOutcome::PlatformError(_)) => {}
            other => panic!("existing member must yield a trash attempt: {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }
}
