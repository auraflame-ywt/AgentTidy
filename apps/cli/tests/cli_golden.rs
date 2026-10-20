//! End-to-end CLI golden baselines (`Start.md` §18.5).
//!
//! Each test lays out a temp home from the existing sanitized fixtures,
//! spawns the compiled `agenttidy` binary with `HOME` / `USERPROFILE`
//! redirected to that temp dir, then asserts that the `--json` envelope
//! matches a committed snapshot. The binary inherits the per-spawn env
//! vars without mutating the test runner's process state.
//!
//! Absolute paths, completion timestamps, the `agent_running` flag, the
//! `platform` label and measured `allocated_bytes` are normalised to
//! placeholders so a single golden set covers macOS and Windows CI runners.
//! Other fixture fields (cwd, session
//! ids, originator, cli_version, the four workspace-safety booleans,
//! `thread-index-unavailable`, `workbuddy-db-unavailable`) survive into the
//! golden verbatim — those are exactly the traces this test locks in.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

const FIXTURES: &str = "../../fixtures";
const GOLDENS_DIR: &str = "golden";

/// One CLI invocation: captured stdout, stderr and exit status.
#[derive(Debug)]
struct CliOutput {
    stdout: String,
    stderr: String,
    status: i32,
}

/// Source fixture path (read-only). `CARGO_MANIFEST_DIR` points at
/// `apps/cli`, so `../../fixtures/...` lands at the repo-root tree.
fn fixture_file(provider: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURES)
        .join(provider)
        .join("sessions")
        .join("sample-session.jsonl")
}

/// Path to a golden JSON snapshot.
fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(GOLDENS_DIR)
        .join(format!("{name}.json"))
}

/// Build a fresh, unique temp directory. The provider tests under
/// `providers/*/src/lib.rs` use the same temp_dir + pid + nanos convention;
/// matching it keeps parallel test runs from colliding.
fn unique_temp_home(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("agenttidy-cli-golden-{label}-{pid}-{nanos}-{seq}"));
    fs::create_dir_all(&path).expect("create temp home");
    path
}

fn lay_codex_fixture(home: &Path) {
    let codex_root = home.join(".codex");
    fs::create_dir_all(codex_root.join("sessions/2026/06/04")).expect("codex sessions tree");
    // The adapter only reads the first JSONL line; the rest of the file
    // (response_item / event_msg / turn_context) is silently ignored, so
    // we copy the sanitized fixture verbatim into the rollout path the
    // existing unit test uses (providers/codex/src/lib.rs:679-702).
    fs::copy(
        fixture_file("codex"),
        codex_root.join("sessions/2026/06/04/rollout-019e927e-a8f2-7e61-8a09-4e486e1215ea.jsonl"),
    )
    .expect("copy codex fixture");
    // Empty default workspace dir exercises the Workspace-resource emission
    // path (providers/codex/src/lib.rs:567-634, gated on the second
    // data_root existing).
    fs::create_dir_all(home.join("Documents").join("Codex")).expect("codex workspace dir");
    // No state_5.sqlite: keeps Codex on its `thread-index-unavailable`
    // degraded warning, which is itself part of the locked contract.
}

fn lay_claude_code_fixture(home: &Path) {
    let claude_root = home.join(".claude");
    let project_dir = claude_root.join("projects/-Users-x-AgentTidy");
    fs::create_dir_all(&project_dir).expect("claude project dir");
    // Filename must equal the first-line sessionId — verified at
    // providers/claude-code/src/lib.rs:227-238.
    fs::copy(
        fixture_file("claude-code"),
        project_dir.join("9521178d-ae3a-4959-b9ac-7591e7eaf5d6.jsonl"),
    )
    .expect("copy claude-code fixture");
}

fn lay_workbuddy_fixture(home: &Path) {
    let workbuddy_root = home.join(".workbuddy");
    let project_dir = workbuddy_root.join("projects/-Users-x-AgentTidy");
    fs::create_dir_all(&project_dir).expect("workbuddy project dir");
    // Same filename-must-match-sessionId rule (providers/workbuddy/src/lib.rs:182).
    fs::copy(
        fixture_file("workbuddy"),
        project_dir.join("d493ed8f-0147-4c01-89df-b9da4b956532.jsonl"),
    )
    .expect("copy workbuddy fixture");
    fs::create_dir_all(home.join("WorkBuddy")).expect("workbuddy workspace dir");
    // No workbuddy.db: keeps the `workbuddy-db-unavailable` warning on
    // inspection; that warning is part of the locked contract.
}

/// Spawn the `agenttidy` binary with HOME/USERPROFILE redirected at the
/// given temp home. Per-spawn `Command::env` keeps the runner process
/// untouched. `LOCALAPPDATA` is redirected too: the Windows Claude
/// Desktop installation is discovered via `LOCALAPPDATA/Claude-3p`, so
/// without this a real machine install leaks into every golden
/// comparison (and breaks the empty-home regressions).
fn run_cli(command: &str, home: &Path) -> CliOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_agenttidy"))
        .arg(command)
        .arg("--json")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("LOCALAPPDATA", home)
        // Never run the developer's installed CLI during fixture snapshots.
        // The missing-binary diagnostic is stable across local/CI machines.
        .env("PATH", home.join("empty-bin"))
        .output()
        .expect("spawn agenttidy binary");
    CliOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    }
}

/// Assert the envelope contract (`agenttidy.cli.v1`, mode, command) and
/// return the parsed `Value` for golden comparison.
fn assert_envelope(output: &CliOutput, expected_command: &str) -> Value {
    assert_eq!(
        output.status, 0,
        "agenttidy {expected_command} --json exited with {}: stderr={}",
        output.status, output.stderr
    );
    let value: Value = serde_json::from_str(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON from agenttidy: {error}\nstdout={}",
            output.stdout
        )
    });
    assert_eq!(value["schema_version"], "agenttidy.cli.v1");
    assert_eq!(value["mode"], "read-only");
    assert_eq!(value["command"], expected_command);
    value
}

/// Rewrite every absolute path under the temp home to `<TEST_HOME>` and
/// flip remaining backslashes to forward slashes, so a single set of
/// goldens covers macOS and Windows. Then zero the per-run `completed_at`
/// timestamp, replace `installation.platform` with `<PLATFORM>` and
/// replace `inspection.agent_running` with `<AGENT_RUNNING>` so dev
/// machines running the agent don't break the baseline.
fn normalize(value: &mut Value, temp_home: &Path) {
    let temp_home_str = temp_home.display().to_string();

    fn walk(value: &mut Value, temp_home_str: &str) {
        match value {
            Value::String(s) => {
                *s = s.replace(temp_home_str, "<TEST_HOME>");
                *s = s.replace('\\', "/");
            }
            Value::Array(items) => items.iter_mut().for_each(|v| walk(v, temp_home_str)),
            Value::Object(map) => {
                // Recurse into values first.
                for v in map.values_mut() {
                    walk(v, temp_home_str);
                }
                // `allocated_bytes` is host-filesystem-dependent (cluster
                // size, NTFS compression, sparse extents) — a measured
                // number cannot be stable across dev machines and CI
                // runners. `null` (provider never measured it) is
                // deterministic and therefore stays verbatim.
                if let Some(value) = map.get_mut("allocated_bytes") {
                    if value.is_number() {
                        *value = Value::String("<ALLOCATED>".into());
                    }
                }
                // Then rewrite keys that embed the temp home — e.g.
                // `readable_roots` in `ProviderInspection`, or future
                // `unreadable:<path>` problem keys. serde_json's Map is
                // BTreeMap-backed; rebuilding in place re-sorts on insert.
                let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
                for (mut k, v) in entries {
                    if k.contains(temp_home_str) {
                        k = k.replace(temp_home_str, "<TEST_HOME>");
                        k = k.replace('\\', "/");
                    }
                    map.insert(k, v);
                }
            }
            _ => {}
        }
    }
    walk(value, &temp_home_str);

    // Zero the per-run completion timestamp on each AgentSnapshot. The
    // `sessions` command returns Vec<Session> directly under `data`
    // (no `completed_at`), so this block is a no-op there.
    if let Value::Object(envelope) = value {
        if let Some(Value::Array(data)) = envelope.get_mut("data") {
            for snapshot in data.iter_mut() {
                let Value::Object(map) = snapshot else {
                    continue;
                };
                if map.contains_key("completed_at") {
                    map.insert("completed_at".into(), Value::Number(0.into()));
                }
                // installation.platform is platform-specific.
                if let Some(Value::Object(installation)) = map.get_mut("installation") {
                    installation.insert("platform".into(), Value::String("<PLATFORM>".into()));
                }
                // inspection.agent_running reflects whatever processes the
                // dev machine happens to have at test time.
                if let Some(Value::Object(inspection)) = map.get_mut("inspection") {
                    inspection.insert(
                        "agent_running".into(),
                        Value::String("<AGENT_RUNNING>".into()),
                    );
                }
                // session.lifecycle is partly process-derived (Claude Code
                // returns Unknown when its binary is detected running) and
                // partly fixture-derived (Codex checks thread-writer-locks/).
                // Pin the field's presence; leave its specific value out so
                // the golden is stable across dev machines and CI runners.
                if let Some(Value::Array(sessions)) = map.get_mut("sessions") {
                    for session in sessions {
                        if let Value::Object(session_map) = session {
                            if let Some(Value::Object(lifecycle)) = session_map.get_mut("lifecycle")
                            {
                                lifecycle
                                    .insert("state".into(), Value::String("<LIFECYCLE>".into()));
                            }
                        }
                    }
                }
            }
        }
        // Sessions command: data is Vec<Session>; normalise its
        // lifecycle.state field too (the top-level block above only
        // reached into AgentSnapshot-shaped entries).
        if let Some(Value::Array(data)) = envelope.get_mut("data") {
            for session in data.iter_mut() {
                if let Value::Object(session_map) = session {
                    if let Some(Value::Object(lifecycle)) = session_map.get_mut("lifecycle") {
                        lifecycle.insert("state".into(), Value::String("<LIFECYCLE>".into()));
                    }
                }
            }
        }
    }
}

/// Compare the actual JSON against the committed golden file. On a first
/// run (no golden present) write the actual JSON to disk and panic with a
/// hint to review and re-run.
fn assert_matches_golden(name: &str, actual: &Value) {
    let path = golden_path(name);
    let actual_text = serde_json::to_string_pretty(actual).expect("serialize actual JSON");

    if !path.exists() {
        fs::create_dir_all(path.parent().unwrap()).expect("create golden dir");
        fs::write(&path, format!("{actual_text}\n")).expect("write golden JSON");
        panic!(
            "blessed golden '{name}' at {} — verify, commit, and re-run cargo test",
            path.display()
        );
    }

    let expected_text = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("read golden {}: {error}", path.display());
    });
    let expected: Value = serde_json::from_str(&expected_text)
        .unwrap_or_else(|error| panic!("invalid golden JSON at {}: {error}", path.display()));
    if expected != *actual {
        eprintln!(
            "=== expected golden ({}) ===\n{expected_text}",
            path.display()
        );
        eprintln!("=== actual ===\n{actual_text}");
        panic!("golden '{name}' mismatch (see diff above)");
    }
}

// ---------------------------------------------------------------------------
// Fixture-driven golden baselines (3 providers × 3 commands = 9).
// ---------------------------------------------------------------------------

#[test]
fn codex_doctor_matches_golden() {
    let home = unique_temp_home("codex-doctor");
    lay_codex_fixture(&home);
    let output = run_cli("doctor", &home);
    let mut value = assert_envelope(&output, "doctor");
    normalize(&mut value, &home);
    assert_matches_golden("codex__doctor", &value);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn codex_scan_matches_golden() {
    let home = unique_temp_home("codex-scan");
    lay_codex_fixture(&home);
    let output = run_cli("scan", &home);
    let mut value = assert_envelope(&output, "scan");
    normalize(&mut value, &home);
    assert_matches_golden("codex__scan", &value);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn codex_sessions_matches_golden() {
    let home = unique_temp_home("codex-sessions");
    lay_codex_fixture(&home);
    let output = run_cli("sessions", &home);
    let mut value = assert_envelope(&output, "sessions");
    normalize(&mut value, &home);
    assert_matches_golden("codex__sessions", &value);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn claude_code_doctor_matches_golden() {
    let home = unique_temp_home("claude-code-doctor");
    lay_claude_code_fixture(&home);
    let output = run_cli("doctor", &home);
    let mut value = assert_envelope(&output, "doctor");
    normalize(&mut value, &home);
    assert_matches_golden("claude-code__doctor", &value);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn claude_code_scan_matches_golden() {
    let home = unique_temp_home("claude-code-scan");
    lay_claude_code_fixture(&home);
    let output = run_cli("scan", &home);
    let mut value = assert_envelope(&output, "scan");
    normalize(&mut value, &home);
    assert_matches_golden("claude-code__scan", &value);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn claude_code_sessions_matches_golden() {
    let home = unique_temp_home("claude-code-sessions");
    lay_claude_code_fixture(&home);
    let output = run_cli("sessions", &home);
    let mut value = assert_envelope(&output, "sessions");
    normalize(&mut value, &home);
    assert_matches_golden("claude-code__sessions", &value);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn workbuddy_doctor_matches_golden() {
    let home = unique_temp_home("workbuddy-doctor");
    lay_workbuddy_fixture(&home);
    let output = run_cli("doctor", &home);
    let mut value = assert_envelope(&output, "doctor");
    normalize(&mut value, &home);
    assert_matches_golden("workbuddy__doctor", &value);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn workbuddy_scan_matches_golden() {
    let home = unique_temp_home("workbuddy-scan");
    lay_workbuddy_fixture(&home);
    let output = run_cli("scan", &home);
    let mut value = assert_envelope(&output, "scan");
    normalize(&mut value, &home);
    assert_matches_golden("workbuddy__scan", &value);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn workbuddy_sessions_matches_golden() {
    let home = unique_temp_home("workbuddy-sessions");
    lay_workbuddy_fixture(&home);
    let output = run_cli("sessions", &home);
    let mut value = assert_envelope(&output, "sessions");
    normalize(&mut value, &home);
    assert_matches_golden("workbuddy__sessions", &value);
    let _ = fs::remove_dir_all(&home);
}

// ---------------------------------------------------------------------------
// Empty-home regressions (3 commands × no agents → empty `data: []`).
// ---------------------------------------------------------------------------

#[test]
fn empty_home_doctor_is_empty_envelope() {
    let home = unique_temp_home("empty-doctor");
    let output = run_cli("doctor", &home);
    let mut value = assert_envelope(&output, "doctor");
    normalize(&mut value, &home);
    assert_matches_golden("empty__doctor", &value);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn empty_home_scan_is_empty_envelope() {
    let home = unique_temp_home("empty-scan");
    let output = run_cli("scan", &home);
    let mut value = assert_envelope(&output, "scan");
    normalize(&mut value, &home);
    assert_matches_golden("empty__scan", &value);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn empty_home_sessions_is_empty_envelope() {
    let home = unique_temp_home("empty-sessions");
    let output = run_cli("sessions", &home);
    let mut value = assert_envelope(&output, "sessions");
    normalize(&mut value, &home);
    assert_matches_golden("empty__sessions", &value);
    let _ = fs::remove_dir_all(&home);
}
