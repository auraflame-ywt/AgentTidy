//! Strict local Windows acceptance (`Start.md` §18.3 / §18.4 / §16.4).
//!
//! The macOS counterpart (`macos_acceptance.rs`) proves that a native
//! trash round trip preserves the sample and can be undone. Windows
//! needs the same proof, and it needs it against the *Windows*
//! Recycle Bin, whose shape is different from the macOS Trash:
//!
//! - a recycled item is stored as a pair of files in
//!   `<volume>:\$Recycle.Bin\<SID>\`: `$I<id>` holds the metadata and
//!   `$R<id>` holds the content;
//! - the **original name is not kept** — the pair is named `$I<id>` /
//!   `$R<id>`, so the sample must be located through the original path
//!   recorded inside the `$I` metadata file;
//! - the `$I` layout is: version (8 bytes), original file size
//!   (8 bytes), deletion FILETIME (8 bytes), path length in bytes
//!   (4 bytes), then the original path as UTF-16LE.
//!
//! Only uniquely named temporary samples are touched. The Recycle Bin
//! is never enumerated for the user's items, never purged, and no
//! provider data is read or cleaned.

use crate::fs_probe::file_identity;
use crate::trash::{move_to_trash, TrashOutcome};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Run explicitly with `--ignored --exact` and a single test thread.
/// Unlike ordinary unit tests, a platform refusal (no Recycle Bin, a
/// denied delete, a locked file) fails this acceptance test — it is
/// never downgraded to a skip.
#[test]
#[ignore = "moves/restores only its own samples through the real Windows Recycle Bin"]
fn native_trash_and_manual_restore_preserve_file_and_directory() {
    let token = format!(
        "agenttidy-win-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = std::env::temp_dir().join(&token);
    std::fs::create_dir(&root).unwrap();

    for is_directory in [false, true] {
        let name = format!(
            "{token} 空格 中文 'quoted'{}.sample",
            if is_directory { "-dir" } else { "-file" }
        );
        let original = root.join(&name);
        let content = b"AgentTidy native acceptance sample - no real session data";
        if is_directory {
            std::fs::create_dir(&original).unwrap();
            std::fs::write(original.join("nested 中文.txt"), content).unwrap();
        } else {
            std::fs::write(&original, content).unwrap();
        }
        let identity = file_identity(&original).expect("sample identity");

        assert_eq!(
            move_to_trash(&original).unwrap(),
            TrashOutcome::Removed,
            "native Recycle Bin refusal is a failed acceptance, never a skip"
        );
        assert!(!original.exists());

        // Locate our own sample inside the Recycle Bin by the original
        // path recorded in the `$I` metadata. The content lives in the
        // paired `$R` file.
        let (info_path, data_path) = find_recycled_sample(&root, &name)
            .expect("our sample must be findable in the Recycle Bin");
        assert_eq!(
            read_recycled_original_path(&info_path).as_deref(),
            Some(original.to_string_lossy().as_ref()),
            "the recycled metadata must record our sample's original path"
        );

        if is_directory {
            assert!(
                data_path.is_dir(),
                "a recycled directory keeps its directory shape"
            );
            let nested = data_path.join("nested 中文.txt");
            assert_eq!(std::fs::read(&nested).unwrap(), content);
        } else {
            assert_eq!(std::fs::read(&data_path).unwrap(), content);
        }

        // Manual restore: move the content back and drop the metadata,
        // which is what a user does in the Recycle Bin UI. Verify the
        // identity of the restored sample, not just its contents.
        std::fs::rename(&data_path, &original).expect("manual move-back from Recycle Bin");
        std::fs::remove_file(&info_path).ok();
        assert_eq!(file_identity(&original), Some(identity));
        let restored = if is_directory {
            original.join("nested 中文.txt")
        } else {
            original.clone()
        };
        assert_eq!(std::fs::read(&restored).unwrap(), content);
        assert!(!data_path.exists());
    }
    std::fs::remove_dir_all(&root).unwrap();
    eprintln!(
        "Windows native Recycle Bin + manual restore: file, nested directory, spaces, CJK and quotes verified"
    );
}

/// Find the `$I`/`$R` pair that records `name` inside `root`.
///
/// The search is limited to the volume that holds `root` and to the
/// current user's own Recycle Bin folder. Other users' bins are not
/// readable and are never touched.
fn find_recycled_sample(root: &Path, name: &str) -> Option<(PathBuf, PathBuf)> {
    let volume = volume_root(root)?;
    let bin = Path::new(&volume).join(format!("$Recycle.Bin\\{}", current_user_sid()?));
    let entries = std::fs::read_dir(&bin).ok()?;
    for entry in entries.flatten() {
        let info = entry.path();
        if !is_recycle_info_file(&info) {
            continue;
        }
        let Some(original) = read_recycled_original_path(&info) else {
            continue;
        };
        // Match on the file name only, and require our unique token to
        // be part of it — never touch an unrelated recycled item.
        if !original.ends_with(name) {
            continue;
        }
        if let Some(data) = recycled_data_path(&info) {
            if data.exists() {
                return Some((info, data));
            }
        }
    }
    None
}

/// The `$R` data file that pairs with a `$I` metadata file.
///
/// The pair keeps the same identifier and extension — `$I81PJRF.sample`
/// pairs with `$R81PJRF.sample` — so only the leading `$I` becomes
/// `$R`.
fn recycled_data_path(info: &Path) -> Option<PathBuf> {
    let name = info.file_name()?.to_str()?;
    let rest = name.strip_prefix("$I")?;
    let data_name = format!("$R{rest}");
    Some(info.with_file_name(data_name))
}

/// A `$I<id>` metadata file (the `<id>` may carry the original
/// extension, e.g. `$I81PJRF.sample`).
fn is_recycle_info_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with("$I"))
}

/// Read the original path recorded in a `$I` metadata file.
///
/// Layout: version (8), original size (8), deletion FILETIME (8),
/// path length in bytes (4), then the path as UTF-16LE.
fn read_recycled_original_path(info: &Path) -> Option<String> {
    let bytes = std::fs::read(info).ok()?;
    // 8 + 8 + 8 + 4 = 28 bytes of header.
    if bytes.len() < 28 {
        return None;
    }
    let units: Vec<u16> = bytes[28..]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    if units.is_empty() {
        return None;
    }
    Some(String::from_utf16_lossy(&units))
}

/// The volume root of `path` (e.g. `C:\`), because the Recycle Bin is
/// per volume.
fn volume_root(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let bytes = text.as_bytes();
    // `C:\...` — a drive letter followed by a colon.
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some(text[..2].to_string());
    }
    // A UNC path (`\\server\share\...`) has no drive-letter volume; the
    // Recycle Bin of a network share is not ours to touch, so report
    // that we cannot locate one.
    None
}

/// The current user's SID, as the Recycle Bin folder is named after it.
fn current_user_sid() -> Option<String> {
    // The Windows API answer is the token's user SID; reading it from
    // `whoami /user` keeps this crate free of extra Win32 surface and
    // is enough for a local acceptance test.
    let output = std::process::Command::new("whoami")
        .arg("/user")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(index) = line.find("S-1-") {
            return Some(line[index..].trim().to_string());
        }
    }
    None
}
