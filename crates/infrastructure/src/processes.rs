//! Process-presence signals (`Start.md` §16.2).
//!
//! "Is the agent running" is exactly one signal among several (recent
//! writes, file locks, scan-before/after fingerprints) — this module
//! deliberately answers only the process question and nothing else.
//! Policy layers combine signals (§16.2); providers surface the fact
//! via `ProviderInspection::agent_running`.

use sysinfo::System;

/// What marks a process as "probably this agent".
///
/// Executable-name matching is exact (stem compared, extension and case
/// ignored — `WorkBuddy.exe` matches `workbuddy`); command-line
/// matching is substring, for the common case of agents running as
/// `node …/claude/…` where the exe name alone says nothing. Matching
/// generously is acceptable: a false "running" only blocks cleanup
/// (fail closed), it never grants anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSignature {
    /// Lowercase executable stems, e.g. `["claude", "workbuddy"]`.
    pub exe_names: Vec<String>,
    /// Lowercase needles searched in the full command line, e.g.
    /// `[".claude", "claude-code"]`.
    pub cmdline_substrings: Vec<String>,
}

impl ProcessSignature {
    pub fn new(exe_names: &[&str], cmdline_substrings: &[&str]) -> Self {
        Self {
            exe_names: exe_names.iter().map(|s| s.to_lowercase()).collect(),
            cmdline_substrings: cmdline_substrings
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
        }
    }
}

/// Match both the display name and executable location: several macOS
/// products ship the main binary as `Electron`, not their product name.
/// Executable paths are used even when command-line access is restricted.
fn matches_process(
    signature: &ProcessSignature,
    name: &str,
    executable: Option<&std::path::Path>,
    command: &[String],
) -> bool {
    let stem = name.to_lowercase().trim_end_matches(".exe").to_string();
    if signature.exe_names.contains(&stem) {
        return true;
    }
    if signature.cmdline_substrings.is_empty() {
        return false;
    }
    let executable = executable
        .map(|path| path.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let command = command.join(" ").to_lowercase();
    signature
        .cmdline_substrings
        .iter()
        .any(|needle| executable.contains(needle) || command.contains(needle))
}

/// Is any process matching one of the signatures currently running?
///
/// This is a point-in-time check; a clean return does *not* prove the
/// agent cannot start mid-operation (the executor re-checks, §12.1).
#[must_use]
pub fn any_process_running(signatures: &[ProcessSignature]) -> bool {
    if signatures.is_empty() {
        return false;
    }
    let mut system = System::new();
    system.refresh_processes();

    // An empty/restricted process snapshot cannot prove that agents are
    // stopped. Refuse cleanup rather than interpreting a probe failure as idle.
    if system
        .process(sysinfo::Pid::from_u32(std::process::id()))
        .is_none()
    {
        return true;
    }
    for process in system.processes().values() {
        if signatures.iter().any(|signature| {
            matches_process(signature, process.name(), process.exe(), process.cmd())
        }) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn some_process_is_running() {
        // `svchost.exe` on Windows and `launchd` on macOS are
        // guaranteed to be running on a normal machine, so matching
        // either by executable name is a stable positive case that
        // does not depend on any specific agent being installed.
        // (`cmd()` is unreliable on Windows: cmdline reads from
        // other users' processes need elevation; we use `exe_name`
        // instead, which is always available.)
        let exe_name = if cfg!(windows) { "svchost" } else { "launchd" };
        let signature = ProcessSignature::new(&[exe_name], &[]);
        assert!(any_process_running(&[signature]));
    }

    #[test]
    fn no_process_matches_garbage_signature() {
        // Nothing on a normal machine runs anything like this.
        let signature = ProcessSignature::new(
            &["definitely-not-a-real-agent-xyzzy"],
            &["definitely-not-a-real-agent-xyzzy"],
        );
        assert!(!any_process_running(&[signature]));
    }

    #[test]
    fn empty_signatures_mean_no() {
        assert!(!any_process_running(&[]));
        // Signature with no criteria must not match everything.
        assert!(!any_process_running(&[ProcessSignature::new(&[], &[])]));
    }
    /// macOS WorkBuddy 5.8.3 reports Electron as CFBundleExecutable.
    /// Other Electron products must not be attributed to WorkBuddy (§16.2).
    #[test]
    fn electron_bundle_identity_is_checked_without_command_line_access() {
        let signature = ProcessSignature::new(&["WorkBuddy"], &["workbuddy.app/contents/"]);
        assert!(matches_process(
            &signature,
            "Electron",
            Some(std::path::Path::new(
                "/Applications/WorkBuddy.app/Contents/MacOS/Electron"
            )),
            &[]
        ));
        assert!(!matches_process(
            &signature,
            "Electron",
            Some(std::path::Path::new(
                "/Applications/Other.app/Contents/MacOS/Electron"
            )),
            &[]
        ));
    }
}
