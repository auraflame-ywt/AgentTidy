//! Platform infrastructure (design doc `Start.md` §13.1, Phase 2).
//!
//! Exposes the platform capabilities behind unified, provider-neutral
//! functions — paths, filesystem probing, disk usage, JSONL, read-only
//! SQLite, process signals and trash. Platform differences (Win32
//! calls, Unix stat semantics, case-sensitivity rules) are confined to
//! `#[cfg]` branches inside these modules; nothing platform-specific
//! leaks out of this crate (red line #11), and no business logic lives
//! here (providers describe facts; policy decides — §10).
//!
//! Usage map (by design-doc section):
//! - §16.1 path safety — [`paths`]
//! - §8 space accounting inputs — [`fs_probe`], [`disk_usage`]
//! - §2.1/§18.1 transcript reading — [`jsonl`]
//! - §13.1 read-only SQLite — [`sqlite`]
//! - §16.2 process signal — [`processes`]
//! - §3.4 trash-first disposal — [`trash`]
//! - §12.1 operation guard — [`operation_guard`]

pub mod audit_log;
pub mod backup;
pub mod disk_usage;
pub mod fs_probe;
pub mod jsonl;
pub mod operation_guard;
pub mod paths;
pub mod processes;
pub mod sqlite;
pub mod trash;
pub mod workspace_safety;

// Strict native tests are opt-in: they trash and restore only their own
// uniquely named samples. Ordinary CI cannot certify native recovery (§18.4).
#[cfg(all(test, target_os = "macos"))]
mod macos_acceptance;
