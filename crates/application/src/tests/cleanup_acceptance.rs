//! Controlled Phase 7 cleanup acceptance on real files
//! (`Start.md` §18.3 / §18.4, `docs/safety/workbuddy-session-fileset.md`).
//!
//! The unit tests around the executor use a mock provider, so they
//! cannot prove that a *real* provider's units survive the real
//! plan -> revalidate -> execute -> recover loop. This module closes that
//! gap: it lays down a synthetic WorkBuddy installation (real directory
//! layout, real SQLite database, real files), drives it through the
//! public Application API, moves the unit to the real OS trash, and
//! restores it by hand.
//!
//! Safety: every file it touches is created by this test in a
//! uniquely named temporary directory. It never reads, moves or deletes
//! a real user session, never enumerates the OS trash beyond locating
//! its own sample, and never purges anything.
//!
//! The test is opt-in (`--ignored --exact`), because it needs the OS
//! trash and writes to the real audit log location.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use agenttidy_core::{CleanupAction, CleanupOutcome, CleanupPlan, ScanOptions};
use agenttidy_provider_api::AgentProviderAdapter;
use agenttidy_workbuddy::WorkBuddyAdapter;

use crate::Application;

/// A synthetic WorkBuddy installation: `projects/<slug>/<id>.jsonl`,
/// the side directory and the `.meta.json` sidecar, plus a
/// `workbuddy.db` whose `sessions` row marks the session as archived.
///
/// The installation lives under an **isolated home** so the whole
/// Application - discovery, planning, revalidation and execution - sees
/// this synthetic installation and nothing else. The revalidator
/// re-discovers installations on its own (§12.2), so a plan built from
/// a hand-made snapshot would be compared against the *real* WorkBuddy
/// install and fail the schema check. Isolating the home is what makes
/// the whole loop run against the synthetic sample.
struct Sample {
    home: PathBuf,
    session_id: String,
    transcript: PathBuf,
    previous_home: Option<std::ffi::OsString>,
    previous_profile: Option<std::ffi::OsString>,
}

impl Sample {
    fn new(tag: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "agenttidy-cleanup-acceptance-{}-{}-{}",
            tag,
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = home.join(".workbuddy");
        let project = root.join("projects/c-sample-project");
        std::fs::create_dir_all(&project).unwrap();
        let session_id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_string();
        let transcript = project.join(format!("{session_id}.jsonl"));
        std::fs::write(
            &transcript,
            format!(
                "{{\"type\":\"message\",\"sessionId\":\"{session_id}\",\"cwd\":\"c:/sample/project\"}}\n"
            ),
        )
        .unwrap();
        // The side directory and the meta sidecar are optional members;
        // include them so the unit carries a real multi-path file set.
        std::fs::create_dir_all(project.join(&session_id).join("tool-results")).unwrap();
        std::fs::write(
            project
                .join(&session_id)
                .join("tool-results")
                .join("result.txt"),
            b"synthetic tool result",
        )
        .unwrap();
        std::fs::write(project.join(format!("{session_id}.meta.json")), "{}").unwrap();

        // The frozen `sessions` v1 contract. `deleted_at` set means
        // Archived, which is the lifecycle the safety doc allows.
        let conn = rusqlite::Connection::open(root.join("workbuddy.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY,
                 cwd TEXT,
                 user_id TEXT,
                 title TEXT,
                 status TEXT,
                 created_at INTEGER,
                 updated_at INTEGER,
                 deleted_at INTEGER,
                 source_mode TEXT,
                 model TEXT,
                 project_id TEXT,
                 buddy_snapshot_id TEXT
             );
             CREATE TABLE automations (id TEXT PRIMARY KEY, cwds TEXT);",
        )
        .unwrap();
        let long_ago = 1_600_000_000_000i64;
        conn.execute(
            "INSERT INTO sessions (id, cwd, status, created_at, updated_at, deleted_at) \
             VALUES (?1, ?2, ?3, ?4, ?4, ?4)",
            rusqlite::params![session_id, "c:/sample/project", "completed", long_ago],
        )
        .unwrap();
        drop(conn);

        // Redirect the home so the Application discovers *this*
        // installation. Both variables are set: `paths::home_dir` reads
        // `USERPROFILE` on Windows and `HOME` elsewhere.
        let previous_home = std::env::var_os("HOME");
        let previous_profile = std::env::var_os("USERPROFILE");
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);

        Self {
            home,
            session_id,
            transcript,
            previous_home,
            previous_profile,
        }
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        for (key, value) in [
            ("HOME", &self.previous_home),
            ("USERPROFILE", &self.previous_profile),
        ] {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        std::fs::remove_dir_all(&self.home).ok();
    }
}

/// The whole beta loop on real files: discover the synthetic install,
/// plan it, revalidate it, execute it into the real OS trash, then
/// restore it by hand and prove the content survived.
#[tokio::test]
#[ignore = "moves a synthetic sample to the real OS trash and restores it"]
async fn workbuddy_file_set_round_trips_through_the_real_trash() {
    let sample = Sample::new("round-trip");
    let app = Application::new();

    // The Application must discover the synthetic installation and
    // recognise the synthetic session.
    let snapshots = app.scan(&ScanOptions::default()).await.unwrap();
    let snapshot = snapshots
        .iter()
        .find(|snapshot| snapshot.installation.id == "workbuddy:default")
        .expect("the synthetic WorkBuddy installation must be discovered");
    assert_eq!(
        snapshot.sessions.len(),
        1,
        "the synthetic session must be recognised"
    );
    assert!(
        snapshot.problems.is_empty(),
        "the synthetic installation must be problem-free: {:?}",
        snapshot.problems
    );

    // The provider must offer a real multi-path unit for it.
    let units = WorkBuddyAdapter
        .build_cleanup_units(snapshot)
        .await
        .unwrap();
    assert_eq!(units.len(), 1, "the synthetic session must form one unit");
    let unit = &units[0];
    assert_eq!(unit.operation.action, CleanupAction::Trash);
    assert!(
        unit.estimated_reclaimable_bytes > 0,
        "the unit must carry a reclaimable estimate"
    );

    // The plan must carry the unit and it must not be blocked.
    let plan = app
        .cleanup_plan_from_snapshots(std::slice::from_ref(snapshot))
        .await
        .unwrap();
    let item = plan
        .items
        .iter()
        .find(|item| item.unit_id == unit.id)
        .expect("unit in plan");
    assert_ne!(
        item.risk,
        agenttidy_core::RiskLevel::Blocked,
        "an archived, exclusive, recoverable synthetic session must not be blocked; reasons: {:?}",
        item.reasons
    );

    // Revalidation must pass for the freshly built plan.
    let revalidated = app.cleanup_revalidate(&plan).await.unwrap();
    assert!(
        revalidated
            .iter()
            .find(|candidate| candidate.unit_id == unit.id)
            .map(|candidate| candidate.outcome != CleanupOutcome::Skipped)
            .unwrap_or(false),
        "the unit must survive revalidation; outcomes: {revalidated:?}"
    );

    // Execution moves the members into the real OS trash.
    let results = app
        .cleanup_execute(&plan, &plan.fingerprint, std::slice::from_ref(&unit.id))
        .await
        .unwrap();
    assert_eq!(
        results[0].outcome,
        CleanupOutcome::Removed,
        "the synthetic unit must be moved to the trash; reason: {:?}",
        results[0].reason
    );
    assert!(
        !sample.transcript.exists(),
        "the transcript must have left its original location"
    );

    // Re-running is an idempotent no-op, never a fresh success.
    let rerun = app
        .cleanup_execute(&plan, &plan.fingerprint, std::slice::from_ref(&unit.id))
        .await
        .unwrap();
    assert_eq!(rerun[0].outcome, CleanupOutcome::Skipped);

    // Recovery is manual and performed by the user: restore the
    // transcript from the trash and prove the content survived.
    let expected = format!(
        "{{\"type\":\"message\",\"sessionId\":\"{}\",\"cwd\":\"c:/sample/project\"}}\n",
        sample.session_id
    );
    restore_from_trash(&sample.transcript, &sample.session_id)
        .expect("the trashed sample must be findable in the OS trash");
    assert_eq!(
        std::fs::read_to_string(&sample.transcript).unwrap(),
        expected,
        "the restored transcript must hold the original bytes"
    );
}

/// Locate a trashed file by its original name and move it back.
///
/// The lookup is deliberately narrow: it only ever inspects the trash
/// of the current user for an entry whose recorded original path ends
/// with the expected file name, and it only acts on that one entry.
#[cfg(target_os = "macos")]
fn restore_from_trash(original: &Path, _session_id: &str) -> Option<PathBuf> {
    let home = agenttidy_infrastructure::paths::home_dir()?;
    let trash = home.join(".Trash");
    let name = original.file_name()?.to_str()?;
    for entry in std::fs::read_dir(&trash).ok()?.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            std::fs::rename(&path, original).ok()?;
            return Some(original.to_path_buf());
        }
    }
    None
}

/// The Windows Recycle Bin stores the original path inside the `$I`
/// metadata file that pairs with the `$R` content file.
#[cfg(target_os = "windows")]
fn restore_from_trash(original: &Path, _session_id: &str) -> Option<PathBuf> {
    let volume = volume_root(original)?;
    let sid = current_user_sid()?;
    let bin = Path::new(&volume).join(format!("$Recycle.Bin\\{sid}"));
    let name = original.file_name()?.to_str()?;
    for entry in std::fs::read_dir(&bin).ok()?.flatten() {
        let info = entry.path();
        let info_name = info.file_name()?.to_str()?;
        if !info_name.starts_with("$I") {
            continue;
        }
        let recorded = read_recycled_original_path(&info)?;
        if !recorded.ends_with(name) {
            continue;
        }
        let data = info.with_file_name(format!("$R{}", &info_name[2..]));
        std::fs::rename(&data, original).ok()?;
        std::fs::remove_file(&info).ok();
        return Some(original.to_path_buf());
    }
    None
}

#[cfg(target_os = "windows")]
fn volume_root(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let bytes = text.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some(text[..2].to_string());
    }
    None
}

#[cfg(target_os = "windows")]
fn read_recycled_original_path(info: &Path) -> Option<String> {
    let bytes = std::fs::read(info).ok()?;
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

#[cfg(target_os = "windows")]
fn current_user_sid() -> Option<String> {
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

/// Keep the unused import warning away on platforms without the
/// matching helper.
#[allow(dead_code)]
fn _assert_plan_type(plan: &CleanupPlan) -> usize {
    plan.items.len()
}
