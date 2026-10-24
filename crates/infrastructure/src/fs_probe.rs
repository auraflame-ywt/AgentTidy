//! Read-only filesystem probing (`Start.md` §8, §16.1).
//!
//! `walk_tree` enumerates a directory tree **without following any
//! link**: symlinks and Windows junctions are reported as
//! [`EntryKind::Link`] entries (the link itself, never its target), and
//! structures the walker does not understand — Windows cloud
//! reparse-point placeholders, Unix FIFOs/sockets — surface as
//! [`EntryKind::Unknown`] so callers can apply "unknown ⇒ Blocked"
//! (red line #6). This is what keeps a Cleanup Unit from escaping the
//! provider data root (§16.1 link boundary).
//!
//! Each file also carries a [`FileIdentity`] (volume + index on
//! Windows, device + inode on Unix) — the key §8 requires for
//! hard-link deduplication in size accounting.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Platform file identity (Windows volume-serial + file-index; Unix
/// device + inode). Two paths with the same identity are the same
/// physical file (hard links) and must be counted once (§8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    pub volume: u64,
    pub index: u64,
}

/// What a walked entry *is*. `Link` and `Unknown` are terminal
/// classifications — the walker never descends past them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    /// Symlink or Windows junction. `target` is the raw link target
    /// when readable; it is recorded, never resolved (§16.1).
    Link {
        target: Option<PathBuf>,
    },
    /// A structure the walker does not understand: Windows reparse
    /// points other than symlinks/junctions (e.g. cloud placeholders)
    /// or Unix special files. Unknown ⇒ caller must fail closed.
    Unknown,
}

/// Summary of one filesystem entry, gathered without following links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    pub path: PathBuf,
    pub kind: EntryKind,
    /// Logical file length (link entries report the link's own length;
    /// directories report 0). Never the target's size.
    pub logical_len: u64,
    /// On-disk allocated bytes when measurable (Windows: compressed
    /// size; Unix: `st_blocks * 512`). `None` = unknown, and size
    /// aggregates must degrade accordingly (§8 no fake precision).
    pub allocated_len: Option<u64>,
    /// File identity for hard-link dedup (§8). `None` when the platform
    /// call failed (e.g. file opened exclusively by an agent) — the
    /// consumer must treat it conservatively.
    pub identity: Option<FileIdentity>,
}

/// Non-fatal walk problem — surfaced, never aborts the walk (fail-closed
/// applies to cleanup, not reporting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkProblem {
    /// Entry exists but could not be stat'd (permissions, concurrent
    /// deletion, …).
    Unreadable { path: PathBuf, error: String },
}

/// Result of [`walk_tree`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WalkOutcome {
    pub entries: Vec<EntryInfo>,
    pub problems: Vec<WalkProblem>,
}

/// Enumerate `root`'s tree read-only.
///
/// Guarantees (§16.1, §8):
/// - links/junctions are returned as [`EntryKind::Link`] and **not**
///   descended into;
/// - unknown structures are not descended into either — the entry is
///   reported so the caller can block on it;
/// - every file carries identity + allocated size when measurable.
///
/// The root itself is the first entry. Unreadable entries are problems,
/// not errors — a partial tree is a reportable fact.
pub fn walk_tree(root: &Path) -> WalkOutcome {
    let mut outcome = WalkOutcome::default();
    let mut iter = WalkDir::new(root)
        // Default is false already; stated explicitly because §16.1
        // depends on this being non-following.
        .follow_links(false)
        .into_iter();

    while let Some(entry) = iter.next() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // WalkDir errors carry a path when one is known.
                let path = e
                    .path()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| root.to_path_buf());
                outcome.problems.push(WalkProblem::Unreadable {
                    path,
                    error: e.to_string(),
                });
                continue;
            }
        };

        let path = entry.path().to_path_buf();
        // symlink_metadata: never resolve the entry — what we classify
        // is what we walk (§16.1).
        let md = match fs::symlink_metadata(&path) {
            Ok(md) => md,
            Err(e) => {
                outcome.problems.push(WalkProblem::Unreadable {
                    path,
                    error: e.to_string(),
                });
                continue;
            }
        };

        let file_type = md.file_type();
        let kind = if file_type.is_symlink() {
            EntryKind::Link {
                target: fs::read_link(&path).ok(),
            }
        } else if is_unclassified_reparse(&md) {
            // Windows: reparse tag std does not treat as a link (e.g.
            // OneDrive cloud placeholders). Unknown ⇒ caller blocks.
            EntryKind::Unknown
        } else if file_type.is_dir() {
            EntryKind::Dir
        } else if file_type.is_file() {
            EntryKind::File
        } else {
            // Unix: FIFOs, sockets, devices. Never descended into;
            // callers fail closed on them.
            EntryKind::Unknown
        };

        // walkdir descends into whatever its own is_dir() accepts. The
        // only entries we classify as non-Dir yet still "look like" dirs
        // to walkdir are Windows cloud-placeholder reparse dirs (std's
        // is_symlink() does not cover their tag, so is_dir() stays
        // true) — opt out of descending into those explicitly.
        if md.is_dir() && !matches!(kind, EntryKind::Dir) && entry.depth() > 0 {
            iter.skip_current_dir();
        }

        let (logical_len, allocated_len, identity) = match &kind {
            EntryKind::File => {
                let logical = md.len();
                (
                    logical,
                    allocated_bytes(&path, logical),
                    file_identity(&path),
                )
            }
            // Links report their own (tiny) metadata; identity of the
            // *target* is explicitly not our business (§16.1).
            EntryKind::Link { .. } => (md.len(), allocated_bytes(&path, md.len()), None),
            _ => (0, None, None),
        };

        outcome.entries.push(EntryInfo {
            path,
            kind,
            logical_len,
            allocated_len,
            identity,
        });
    }

    outcome
}

/// Windows: a reparse-point attribute that std's `is_symlink()` does
/// not classify (junctions/symlinks do classify — cloud placeholders
/// and similar tags do not). Always false elsewhere.
#[cfg(windows)]
fn is_unclassified_reparse(md: &fs::Metadata) -> bool {
    // file_attributes() is Windows' MetadataExt (attributes, not the
    // resolved file kind — symlink_metadata keeps reparse flags set).
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    (md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 && !md.file_type().is_symlink()
}

#[cfg(not(windows))]
fn is_unclassified_reparse(_md: &fs::Metadata) -> bool {
    false
}

/// On-disk allocated bytes for a file.
///
/// Windows: `GetCompressedFileSizeW` — actual cluster usage, honors
/// sparse/compressed files. Unix: `st_blocks * 512` (POSIX 512-byte
/// block convention). `None` when the platform call fails so callers
/// can degrade confidence (§8) instead of guessing.
#[must_use]
pub fn allocated_bytes(path: &Path, _logical: u64) -> Option<u64> {
    #[cfg(windows)]
    {
        windows_allocated_bytes(path)
    }
    #[cfg(not(windows))]
    {
        unix_allocated_bytes(path)
    }
}

#[cfg(windows)]
fn windows_allocated_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Storage::FileSystem::{GetCompressedFileSizeW, INVALID_FILE_SIZE};

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut high: u32 = 0;
    // SAFETY: wide is NUL-terminated and high is a valid out-pointer.
    let low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut high) };
    if low == INVALID_FILE_SIZE {
        // INVALID_FILE_SIZE (0xFFFFFFFF) is either an error or a
        // legitimate low dword; GetLastError distinguishes them.
        // SAFETY: thread-local last-error read.
        if unsafe { GetLastError() } != 0 {
            return None;
        }
    }
    Some((u64::from(high) << 32) | u64::from(low))
}

#[cfg(not(windows))]
fn unix_allocated_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|md| md.blocks() * 512)
}

/// Stable file identity, or `None` when it cannot be read (locked file,
/// race with deletion — the consumer degrades, never guesses).
#[must_use]
pub fn file_identity(path: &Path) -> Option<FileIdentity> {
    #[cfg(windows)]
    {
        windows_file_identity(path)
    }
    #[cfg(not(windows))]
    {
        unix_file_identity(path)
    }
}

#[cfg(windows)]
fn windows_file_identity(path: &Path) -> Option<FileIdentity> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);

    // SAFETY: all pointers below are valid; the handle is closed on
    // every path. OPEN_REPARSE_POINT opens the link itself, matching
    // the non-following walk (§16.1). Sharing all modes so an agent
    // with the file open does not cause us to fail.
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        // All-POD Win32 struct; zero-init is the canonical zero value.
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        let ok = GetFileInformationByHandle(handle, &mut info);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        // BY_HANDLE_FILE_INFORMATION's 64-bit file index is unique on
        // NTFS. (ReFS uses 128-bit ids that this struct truncates —
        // acceptable for v0.1, noted in Start.md §16.1 scope.)
        Some(FileIdentity {
            volume: u64::from(info.dwVolumeSerialNumber),
            index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        })
    }
}

#[cfg(not(windows))]
fn unix_file_identity(path: &Path) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let md = fs::symlink_metadata(path).ok()?;
    Some(FileIdentity {
        volume: md.dev(),
        index: md.ino(),
    })
}

/// Collect the set of identities seen among `entries` — useful for
/// tests and (later) cross-snapshot dedup diagnostics.
#[must_use]
pub fn identities_of(entries: &[EntryInfo]) -> HashSet<FileIdentity> {
    entries.iter().filter_map(|e| e.identity).collect()
}

/// A cleanup-owned file must have no other hard-link aliases (§8 / §16.1).
/// Uninvestigated platforms cannot establish this fact and fail closed.
pub fn is_single_link_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(path)
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.nlink() == 1)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique temp dir per test (std-only; no tempfile dep yet).
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agenttidy-fsprobe-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn walk_reports_files_dirs_and_sizes() {
        let root = temp_root("plain");
        std::fs::write(root.join("a.txt"), vec![0u8; 100]).unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("b.bin"), vec![1u8; 50]).unwrap();

        let out = walk_tree(&root);
        assert!(out.problems.is_empty());
        let files: Vec<&EntryInfo> = out
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .collect();
        assert_eq!(files.len(), 2);

        let a = files.iter().find(|e| e.path.ends_with("a.txt")).unwrap();
        assert_eq!(a.logical_len, 100);
        // Allocated size is measurable on all supported platforms.
        assert!(a.allocated_len.is_some());
        assert!(a.identity.is_some());

        let dirs: Vec<&EntryInfo> = out
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::Dir)
            .collect();
        // Root + sub.
        assert_eq!(dirs.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn walk_does_not_follow_symlinks() {
        let root = temp_root("symlink");
        std::fs::create_dir(root.join("real")).unwrap();
        std::fs::write(root.join("real").join("secret.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();

        let out = walk_tree(&root);
        // The link is reported as a Link entry…
        let link = out
            .entries
            .iter()
            .find(|e| e.path.ends_with("link"))
            .expect("link entry present");
        assert!(matches!(link.kind, EntryKind::Link { .. }));
        assert_eq!(link.identity, None, "links carry no file identity");
        // …and its target's files are NOT part of the walk (§16.1).
        assert!(
            !out.entries
                .iter()
                .any(|e| e.path.ends_with("secret.txt") && e.path.starts_with(root.join("link"))),
            "walker must not descend into the symlink"
        );
        // The real dir still yields its file exactly once.
        assert_eq!(
            out.entries
                .iter()
                .filter(|e| e.path.ends_with("secret.txt"))
                .count(),
            1
        );
    }

    #[cfg(windows)]
    #[test]
    fn walk_does_not_follow_junctions() {
        let root = temp_root("junction");
        let target = root.join("real");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("secret.txt"), b"x").unwrap();

        // Junctions can be created without admin rights (`mklink /J`),
        // unlike symlinks.
        let link = root.join("link");
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .expect("cmd starts");
        if !status.status.success() {
            // Environment cannot create junctions (unusual) — skip
            // rather than fail: this is an environment capability test.
            eprintln!("skipping: mklink /J failed");
            return;
        }

        let out = walk_tree(&root);
        let link_entry = out
            .entries
            .iter()
            .find(|e| e.path.ends_with("link"))
            .expect("junction entry present");
        assert!(
            matches!(link_entry.kind, EntryKind::Link { .. }),
            "junction must classify as Link, got {:?}",
            link_entry.kind
        );
        // §16.1: the walker must not descend through the junction —
        // only the single real copy of the file may appear.
        assert_eq!(
            out.entries
                .iter()
                .filter(|e| e.path.ends_with("secret.txt"))
                .count(),
            1
        );
    }

    #[test]
    fn hardlinks_share_identity_but_walk_twice() {
        // §8: hard links are two directory entries for one file; the
        // walker reports both (it must not hide entries), and the two
        // entries carry the SAME identity so size accounting can dedup.
        let root = temp_root("hardlink");
        let a = root.join("a.txt");
        std::fs::write(&a, vec![7u8; 64]).unwrap();
        let b = root.join("b.txt");
        std::fs::hard_link(&a, &b).expect("hard link creation (NTFS/APFS support this)");

        let out = walk_tree(&root);
        let mut files = out
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .filter(|e| e.path.ends_with("a.txt") || e.path.ends_with("b.txt"));
        let (a_info, b_info) = (files.next().expect("a"), files.next().expect("b"));
        assert!(a_info.identity.is_some() && b_info.identity.is_some());
        assert_eq!(a_info.identity, b_info.identity, "same physical file");
        // And the identity set collapses them to one.
        assert_eq!(identities_of(&out.entries).len(), 1);
    }

    #[test]
    fn missing_root_is_reported_not_panicked() {
        let root = temp_root("missing");
        let missing = root.join("does-not-exist");
        let out = walk_tree(&missing);
        // WalkDir surfaces the root error as a problem entry.
        assert!(!out.problems.is_empty());
    }
}
