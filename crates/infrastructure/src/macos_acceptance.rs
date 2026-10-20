//! Strict local macOS acceptance (`Start.md` §18.3 / §18.4 / §16.4).
//! Only uniquely named temporary samples are touched. No Trash enumeration,
//! purge, Agent shutdown or real provider cleanup is performed.

use crate::fs_probe::file_identity;
use crate::trash::{move_to_trash, TrashOutcome};
use std::time::{SystemTime, UNIX_EPOCH};

/// Run explicitly with `--ignored --exact` and a single test thread. Unlike
/// ordinary unit tests, platform/TCC/sandbox errors fail this acceptance test.
#[test]
#[ignore = "moves/restores only its own samples through the real macOS Trash"]
fn native_trash_and_manual_restore_preserve_file_and_directory() {
    let token = format!(
        "agenttidy-mac-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = std::env::temp_dir().join(&token);
    std::fs::create_dir(&root).unwrap();
    let trash = crate::paths::home_dir()
        .expect("local user home")
        .join(".Trash");
    for is_directory in [false, true] {
        let name = format!(
            "{token} 空格 中文 'quoted'{}.sample",
            if is_directory { "-dir" } else { "-file" }
        );
        let original = root.join(&name);
        let destination = trash.join(&name);
        assert!(
            !destination.exists(),
            "test token must not collide with an existing Trash entry"
        );
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
            "native Trash refusal is a failed acceptance, never a skip"
        );
        assert!(!original.exists());
        // Exact path + identity prove this is our sample; never inspect or
        // restore another user's Trash item, even after a name collision.
        assert_eq!(file_identity(&destination), Some(identity));
        assert!(
            !original.exists(),
            "never overwrite a replacement during restore"
        );
        std::fs::rename(&destination, &original).expect("manual move-back from Trash");
        assert_eq!(file_identity(&original), Some(identity));
        let restored = if is_directory {
            original.join("nested 中文.txt")
        } else {
            original
        };
        assert_eq!(std::fs::read(restored).unwrap(), content);
        assert!(!destination.exists());
    }
    std::fs::remove_dir_all(&root).unwrap();
    eprintln!("macOS native Trash + manual restore: file, nested directory, spaces, CJK and quotes verified");
}
