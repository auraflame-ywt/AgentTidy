//! Provider adapter contracts (`Start.md` §10).
//!
//! This crate defines the `AgentProviderAdapter` interface: the boundary
//! between AgentTidy core and per-agent providers. Providers describe
//! facts, resource relationships, operations and preconditions — they
//! never decide cleanup policy (red lines #2, #3).
//!
//! Phase 1 status: the trait shape and inspection types are defined;
//! adapter implementations land in Phase 3 (`providers/` crates) on top of
//! infrastructure services (Phase 2). Cleanup-unit methods
//! (`build_cleanup_units` / `validate_cleanup_unit`, §10) landed in
//! Phase 6 and are enabled per resource type in Phase 7 — they are
//! intentionally absent from providers that have not passed their
//! safety gate so the v0.1 read-only API surface cannot grow execution
//! semantics prematurely.

use agenttidy_core::cleanup::CleanupItemOutcome;
use agenttidy_core::snapshot::ScanProblem;
use agenttidy_core::{
    AgentCapabilities, AgentInstallation, AgentSnapshot, CleanupPrecondition, CleanupUnit,
    ProviderId, ScanOptions,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Result of `AgentProviderAdapter::inspect` (`Start.md` §10.1).
///
/// Facts discovered *before* any scan: version, data roots, schema state,
/// process state, unknown structures. If inspection fails a check,
/// capabilities must degrade accordingly (e.g. running agent ⇒ `ReadOnly`
/// or `Degraded` for topics that would need quiescence).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInspection {
    /// Agent-reported version (may differ from installation version).
    pub version: Option<String>,
    /// Schema versions of agent databases, keyed by a short stable name
    /// (e.g. `state_5`). Absent for file-only providers (Claude Code).
    pub schema_versions: BTreeMap<String, String>,
    /// Journal modes of agent databases, keyed by the same short stable
    /// names as `schema_versions` (§10.1 lists this as a pre-scan fact —
    /// WAL vs rollback journal affects what a safe read looks like).
    /// Absent for file-only providers.
    pub journal_modes: BTreeMap<String, String>,
    /// Whether an agent process is currently running.
    ///
    /// True ⇒ providers must mark active sessions and refuse topics that
    /// need a quiescent state (§16.2 active-data protection).
    pub agent_running: bool,
    /// Read permission state per data root (root path → readable?). Keys
    /// are the installation's `data_roots` — inspection validates them,
    /// it does not rediscover roots (§10.1 "数据根目录" item).
    pub readable_roots: BTreeMap<String, bool>,
    /// Structures the provider did not recognize, with locations —
    /// these feed the "unknown ⇒ Blocked" rule (red line #6).
    pub unknown_structures: Vec<String>,
    /// Non-fatal inspection problems (code → problem).
    pub problems: BTreeMap<String, ScanProblem>,
}

/// Contract implemented by every agent provider adapter.
///
/// All methods are `async` because detection and scanning touch the
/// filesystem and SQLite (read-only). Adapters must be side-effect free:
/// they only *read* agent data.
///
/// `async_trait` (boxed `Send` futures) keeps the trait dyn-compatible, so
/// the application-layer registry can hold `Box<dyn AgentProviderAdapter>`
/// across threads (Tauri commands run on a multithreaded runtime).
///
/// Phase 6 added two `cleanup_*` methods. Default impls return an empty
/// vec — providers that opt into cleanup (per resource type, behind a
/// `docs/safety/` doc per `Start.md` §19) override them. The application
/// policy derives risk on the units the provider emits; cleanup-unit
/// method bodies must stay provider-neutral (no per-provider risk logic,
/// no side effects, no in-place mutation of agent state — red line #3).
#[async_trait::async_trait]
pub trait AgentProviderAdapter: Send + Sync {
    /// Which agent product this adapter implements.
    fn id(&self) -> ProviderId;

    /// Human-readable provider name for UI surfaces.
    fn display_name(&self) -> &str;

    /// Find installations of this agent on this machine.
    ///
    /// Must include *all* data roots per installation (state roots and
    /// default workspace roots, e.g. Codex `~/.codex` +
    /// `~/Documents/Codex`) — see provider investigation docs.
    ///
    /// Returns an empty vec when the agent is simply not installed
    /// (not an error).
    async fn detect(&self) -> anyhow::Result<Vec<AgentInstallation>>;

    /// Pre-scan inspection (§10.1): version, schema, permissions, running
    /// state, unknown structures. Capability derivation happens after
    /// this, not during detection.
    async fn inspect(&self, installation: &AgentInstallation)
        -> anyhow::Result<ProviderInspection>;

    /// Derive capabilities from an inspection result (§9.2). Providers
    /// report *facts as capability states*; they never encode policy
    /// (no "should clean" logic here).
    async fn capabilities(
        &self,
        inspection: &ProviderInspection,
    ) -> anyhow::Result<AgentCapabilities>;

    /// Scan one installation into a read-only snapshot: sessions,
    /// resources, sizes, problems. This is the Phase 3 read-only core loop.
    async fn scan(
        &self,
        installation: &AgentInstallation,
        options: &ScanOptions,
    ) -> anyhow::Result<AgentSnapshot>;

    /// Construct atomic cleanup units from a scan snapshot
    /// (`Start.md` §9.5 + §11). The default returns empty so providers
    /// that have not opted into cleanup keep their old surface. Only
    /// `Exclusive` resources can form a unit — Shared/Unknown inputs
    /// must be filtered here, not at policy time. Units are *structural*
    /// facts (paths, fingerprints, preconditions); whether a unit should
    /// be cleaned is always the policy's call.
    async fn build_cleanup_units(
        &self,
        snapshot: &AgentSnapshot,
    ) -> anyhow::Result<Vec<CleanupUnit>> {
        // Default: no cleanup-enabled resource types for this provider.
        let _ = snapshot;
        Ok(Vec::new())
    }

    /// Validate a unit this adapter produced and return the typed
    /// preconditions the policy / revalidator must check before
    /// executing (e.g. "no symlink outside root", "schema unchanged").
    /// Mirrors §12.2 invalidation triggers in typed form.
    ///
    /// Called by the Application layer at plan-build time; `Err` drops
    /// the unit's plan (fail-closed). Implementations must enforce
    /// §16.1 path safety here: canonicalized containment within the
    /// installation's `data_roots`, no empty / root / home / over-broad
    /// paths, no reserved components. The installation is passed
    /// precisely so the data root is known — validating without it
    /// cannot prove containment. Default returns empty (no provider-
    /// specific preconditions).
    async fn validate_cleanup_unit(
        &self,
        unit: &CleanupUnit,
        installation: &AgentInstallation,
    ) -> anyhow::Result<Vec<CleanupPrecondition>> {
        // Default: no cleanup-enabled resource types for this provider.
        let _ = (unit, installation);
        Ok(Vec::new())
    }

    /// Execute a single `CleanupAction::ProviderOperation` item that
    /// this adapter emitted (`docs/safety/provider-operation.md`,
    /// `Start.md` §12.4). The Application layer routes every
    /// `ProviderOperation` item to this method once the revalidation
    /// passes and the cross-process operation guard is held. Each
    /// provider implements its own coordinated delete — backup →
    /// transaction → FK check → atomic replace — and reports the
    /// outcome (including `reclaimed_bytes` for the audit log).
    ///
    /// Default returns `Failed` so providers that have not opted into
    /// `ProviderOperation` keep the legacy "not wired" behaviour
    /// without breaking dispatch. A provider may use a DbRow locator or an
    /// explicitly validated FileSet for a coordinated multi-store operation;
    /// only the provider interprets its membership, never the shell.
    async fn execute_provider_operation(
        &self,
        item: &agenttidy_core::cleanup::CleanupItem,
        installation: &AgentInstallation,
    ) -> anyhow::Result<CleanupItemOutcome> {
        let _ = (item, installation);
        Ok(CleanupItemOutcome {
            unit_id: item.unit_id.clone(),
            outcome: agenttidy_core::cleanup::CleanupOutcome::Failed,
            reclaimed_bytes: 0,
            reason: Some("ProviderOperation is not supported by this provider".into()),
        })
    }
}

/// §16.1 path safety shared by `file-set` cleanup units across
/// providers lives in `agenttidy_infrastructure::paths::validate_transcript_fileset_paths`
/// (extracted from `providers/claude-code/src/lib.rs::validate_cleanup_unit`).
/// Provider-specific rules — Codex's DB cross-check, WorkBuddy's
/// `sessions.deleted_at` consistency — still live in the provider's
/// own `validate_cleanup_unit` after that helper returns.
#[cfg(test)]
mod tests {
    use super::*;

    /// Inspection model must roundtrip for golden-test serialization
    /// (§18.5) and the future diagnostics view.
    #[test]
    fn inspection_serde_roundtrip() {
        let inspection = ProviderInspection {
            version: Some("2.1.235".into()),
            schema_versions: BTreeMap::from([("workbuddy.db".into(), "0007".into())]),
            journal_modes: BTreeMap::from([("workbuddy.db".into(), "wal".into())]),
            agent_running: true,
            readable_roots: BTreeMap::from([("~/.claude".into(), true)]),
            unknown_structures: vec!["~/.claude/daemon/odd-dir".into()],
            problems: BTreeMap::new(),
        };
        let json = serde_json::to_string(&inspection).unwrap();
        let back: ProviderInspection = serde_json::from_str(&json).unwrap();
        assert_eq!(back, inspection);
        assert!(json.contains("agent_running"));
    }

    /// The trait must stay dyn-compatible: the application-layer registry
    /// holds `Box<dyn AgentProviderAdapter>` (see `crates/core/src/registry.rs`
    /// and Start.md §13). Regressions here surface as E0038 at wiring time.
    #[test]
    fn adapter_trait_is_dyn_compatible() {
        // Type-level assertion: coercing a reference to the trait object
        // only compiles when the trait is dyn-compatible.
        fn _assert_dyn_compatible(_: &(dyn AgentProviderAdapter + Send + Sync)) {}
        fn _assert_boxed(_: Box<dyn AgentProviderAdapter>) {}
        let _ = _assert_dyn_compatible;
        let _ = _assert_boxed;
    }

    /// Default `execute_provider_operation` must keep the legacy
    /// "not wired" behaviour for providers that have not opted in
    /// (Phase 7: only Codex overrides; Claude Code and WorkBuddy
    /// keep the default). Application-layer dispatch is a no-op for
    /// these providers but must still report `Failed` (never
    /// `Removed` / never silently dropped).
    #[tokio::test]
    async fn execute_provider_operation_default_returns_failed() {
        use agenttidy_core::cleanup::{
            CleanupAction, CleanupItem, CleanupLocator, FileIdentity, ResourceFingerprint,
            RiskLevel,
        };
        use agenttidy_core::installation::{InstallationStatus, Platform};
        use agenttidy_core::provider::ProviderId;
        use std::path::PathBuf;

        struct Stub;
        #[async_trait::async_trait]
        impl AgentProviderAdapter for Stub {
            fn id(&self) -> ProviderId {
                ProviderId::new("stub")
            }
            fn display_name(&self) -> &str {
                "stub"
            }
            async fn detect(&self) -> anyhow::Result<Vec<AgentInstallation>> {
                Ok(Vec::new())
            }
            async fn inspect(&self, _: &AgentInstallation) -> anyhow::Result<ProviderInspection> {
                Ok(ProviderInspection {
                    version: None,
                    schema_versions: BTreeMap::new(),
                    journal_modes: BTreeMap::new(),
                    agent_running: false,
                    readable_roots: BTreeMap::new(),
                    unknown_structures: Vec::new(),
                    problems: BTreeMap::new(),
                })
            }
            async fn capabilities(
                &self,
                _: &ProviderInspection,
            ) -> anyhow::Result<AgentCapabilities> {
                Ok(AgentCapabilities::empty())
            }
            async fn scan(
                &self,
                _: &AgentInstallation,
                _: &ScanOptions,
            ) -> anyhow::Result<AgentSnapshot> {
                Ok(AgentSnapshot {
                    installation: AgentInstallation {
                        id: "stub".into(),
                        provider: ProviderId::new("stub"),
                        platform: Platform::Macos,
                        version: None,
                        data_roots: Vec::new(),
                        status: InstallationStatus::Available,
                    },
                    sessions: Vec::new(),
                    resources: Vec::new(),
                    problems: BTreeMap::new(),
                    completed_at: 0,
                })
            }
        }

        let item = CleanupItem {
            evidence: Default::default(),
            unit_id: "stub:row:1".into(),
            installation_id: "stub".into(),
            provider: ProviderId::new("stub"),
            fingerprint: ResourceFingerprint {
                path: PathBuf::from("/tmp/db.sqlite"),
                identity: FileIdentity {
                    volume: 0,
                    index: 0,
                },
                size_bytes: 0,
                mtime_ms: 0,
            },
            locator: CleanupLocator::DbRow {
                database: PathBuf::from("/tmp/db.sqlite"),
                record_id: "row-1".into(),
            },
            estimated_reclaimable_bytes: 0,
            action: CleanupAction::ProviderOperation,
            risk: RiskLevel::ReviewRequired,
            reasons: vec![],
            preconditions: vec![],
        };
        let installation = AgentInstallation {
            id: "stub".into(),
            provider: ProviderId::new("stub"),
            platform: Platform::Macos,
            version: None,
            data_roots: Vec::new(),
            status: InstallationStatus::Available,
        };

        let outcome = Stub
            .execute_provider_operation(&item, &installation)
            .await
            .expect("default impl must not error");
        assert_eq!(
            outcome.outcome,
            agenttidy_core::cleanup::CleanupOutcome::Failed
        );
        assert_eq!(outcome.reclaimed_bytes, 0);
        assert!(outcome
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("not supported"));
    }
}
