//! End-to-end coverage for investigated read-only providers (§18, §22).
//! Real sanitized metadata flows through discovery, the static Application
//! registry and the CLI wire contract; no developer data root is consulted.
//!
//! Platform-neutral by design: the test drives the providers through
//! environment variables and a temporary home, so Windows runs the same
//! wire-contract coverage the Mac developer machine does (D12). The one
//! exception is Pi, which has no Windows sample yet — until one exists
//! its discovery stays disabled there (red line: samples before code).
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "agenttidy-new-providers-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn run(&self, command: &str, pi_root: Option<&std::path::Path>) -> Value {
        let mut process = Command::new(env!("CARGO_BIN_EXE_agenttidy"));
        process
            .args([command, "--json"])
            .env("HOME", &self.0)
            .env("USERPROFILE", &self.0)
            .env("PATH", self.0.join("empty-bin"))
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("PI_CODING_AGENT_SESSION_DIR");
        if let Some(root) = pi_root {
            process.env("PI_CODING_AGENT_DIR", root);
        }
        let output = process.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["mode"], "read-only");
        json["data"].clone()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}
#[test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Pi has no Windows sample yet; its discovery stays disabled there"
)]
fn pi_default_and_configured_roots_reach_the_public_api() {
    for configured in [false, true] {
        let home = Home::new();
        let root = home
            .0
            .join(if configured { "custom-pi" } else { ".pi/agent" });
        fs::create_dir_all(root.join("sessions/project")).unwrap();
        fs::write(
            root.join("sessions/project/session.jsonl"),
            include_str!("../../../fixtures/pi/sessions/header.jsonl"),
        )
        .unwrap();
        let override_root = configured.then_some(root.as_path());
        let scan = home.run("scan", override_root);
        assert_eq!(scan[0]["installation"]["provider"], "pi");
        assert_eq!(scan[0]["sessions"].as_array().unwrap().len(), 1);
        assert!(matches!(
            scan[0]["sessions"][0]["lifecycle"]["state"].as_str(),
            Some("unknown" | "inactive")
        ));
        let doctor = home.run("doctor", override_root);
        assert_eq!(doctor[0]["capabilities"]["sessions"], "read-only");
        assert!(matches!(
            doctor[0]["capabilities"]["cleanup"].as_str(),
            Some("supported" | "read-only")
        ));
        let sessions = home.run("sessions", override_root);
        assert_eq!(sessions.as_array().unwrap().len(), 1);
    }
}
#[test]
fn antigravity_index_reaches_the_public_api_with_cleanup_absent() {
    let home = Home::new();
    let root = home.0.join(".gemini/antigravity");
    fs::create_dir_all(root.join("conversations")).unwrap();
    fs::create_dir_all(root.join("brain")).unwrap();
    let db = rusqlite::Connection::open(root.join("conversation_summaries.db")).unwrap();
    db.execute_batch(include_str!(
        "../../../fixtures/antigravity/index/schema.sql"
    ))
    .unwrap();
    let row: Value =
        serde_json::from_str(include_str!("../../../fixtures/antigravity/index/row.json")).unwrap();
    db.execute("INSERT INTO conversation_summaries (conversation_id,title,last_modified_time,last_user_input_time,workspace_uris,status) VALUES (?1,?2,?3,?3,?4,?5)",rusqlite::params![row["conversation_id"].as_str().unwrap(),row["title"].as_str().unwrap(),row["last_modified_time"].as_str().unwrap(),row["workspace_uris"].as_str().unwrap(),row["status"].as_str().unwrap()]).unwrap();
    drop(db);
    let scan = home.run("scan", None);
    assert_eq!(scan[0]["installation"]["provider"], "antigravity");
    assert_eq!(scan[0]["sessions"][0]["title"], "Sanitized session");
    assert!(scan[0]["problems"]
        .as_object()
        .unwrap()
        .keys()
        .any(|k| k.starts_with("antigravity-transcript-missing:")));
    let doctor = home.run("doctor", None);
    assert_eq!(doctor[0]["capabilities"]["sessions"], "read-only");
    assert!(doctor[0]["capabilities"]["cleanup"].is_null());
}
