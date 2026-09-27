//! Cleanup model — `Start.md` §11 (Cleanup Policy), §12 (CleanupPlan &
//! execution protocol), §16.3 (audit log). Phase 6 shipped the
//! *framework*: types, provider contract additions, Application API
//! surface and the IPC + GUI plumbing.
//!
//! Phase 7 enables cleanup per resource type, each behind its own
//! safety doc. Today only the Claude Code transcript `FileSet` is
//! enabled (`docs/safety/session-transcript-fileset.md`); Codex and
//! WorkBuddy still return no units — per
//! `docs/safety/workspace-cleanup.md:13-15` and `Start.md` §2.3,
//! `~/Documents/Codex/` is user-mixed and Codex session JSONLs leave
//! orphan rows in `state_5.sqlite.threads`; those land in later Phase 7
//! slices with their own per-resource-type safety docs.

use crate::provider::ProviderId;
use crate::resource::{ResourceKind, ResourceRef};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Three-level risk vocabulary per `Start.md` §7 + §11. `Blocked` is the
/// fail-closed default; `LowRisk` requires every §7.1 condition to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RiskLevel {
    /// All §7.1 conditions hold: known kind, known schema, atomic unit,
    /// exclusive ownership, no unknown deps, inactive, recoverable,
    /// re-checkable preconditions.
    LowRisk,
    /// Per §7.2: stale, expensive recovery, or judgement required. Surfaced
    /// to the user with a reason; default selection is unchecked.
    ReviewRequired,
    /// Per §7.3 + §22 red lines: active, unknown, shared, link-crossing,
    /// user-project, DB row, schema mismatch, etc. Non-selectable.
    Blocked,
}

/// What shape of artifact a [`CleanupUnit`] describes. Per `Start.md` §9.5
/// the provider decides which kind applies; the policy and executor are
/// kind-aware (e.g. `FileTree` ⇒ whole-directory atomic move; `FileSet`
/// ⇒ multi-path move; `ProviderOperation` ⇒ never "Move to Trash" — the
/// provider executes an internal backup-then-vacuum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupUnitKind {
    /// A directory treated as one resource (workspace kind, today).
    FileTree,
    /// A set of files forming one logical resource (transcript + sidecars).
    FileSet,
    /// A provider-coordinated operation (DB row delete, journal vacuum).
    /// Never branded "Move to Trash" (red line #8).
    ProviderOperation,
}

/// What executor step the unit runs.
///
/// `Trash` ⇒ OS Recycle Bin / Finder Trash. `ProviderOperation` ⇒ the
/// owning provider's documented operation (Phase 7 — not constructed yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupAction {
    Trash,
    ProviderOperation,
}

/// Per-item outcome after execution. Per `Start.md` §12.3 every item is
/// recorded independently so partial failure does not corrupt the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupOutcome {
    Pending,
    Removed,
    Skipped,
    Failed,
}

/// Precondition kinds. One per `Start.md` §12.2 invalidation trigger so
/// the executor can report *which* trigger invalidated a plan item, not
/// just "fingerprint mismatch". The provider populates the list at unit
/// construction; the policy / revalidator consults it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupPreconditionKind {
    /// The unit's path still resolves to a real on-disk entry.
    PathExists,
    /// `safe_canonicalize` returns the same path the plan was built from
    /// (§16.1 verbatim/UNC asymmetry and `\\?\` quirks).
    PathUnchanged,
    /// The file's `FileIdentity` (volume + index) matches the snapshot.
    IdentityUnchanged,
    /// `st_size` / `GetCompressedFileSize` matches the snapshot.
    SizeUnchanged,
    /// `mtime` matches the snapshot within tolerance.
    MtimeUnchanged,
    /// No agent process detected at execute time (§16.2).
    NoAgentRunning,
    /// The owning session's lifecycle is still `Inactive` / `Archived`
    /// (not `Active`, not `Unknown`).
    SessionInactive,
    /// The workspace still maps 1:1 to exactly one session cwd match.
    SessionStillExclusive,
    /// `provider.inspect` reports the same schema versions / journal
    /// modes as the snapshot used for the plan.
    SchemaUnchanged,
    /// For Codex: `~/.codex/thread-writer-locks/` is empty.
    WriterLockAbsent,
    /// The four workspace-safety booleans
    /// (`filesystem_eligible`, `has_git_marker`, `has_unsafe_entry`,
    /// `has_unreadable_entry`) are unchanged.
    FilesystemEligible,
    /// No entry in the unit's tree crosses the link boundary (no
    /// symlink/junction pointing outside the unit root).
    NoSymlinkOutsideRoot,
}

/// A typed revalidation trigger or admission rule. Provider emits these
/// when constructing a [`CleanupUnit`]; the revalidator walks them and
/// reports each result individually.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupPrecondition {
    pub kind: CleanupPreconditionKind,
    pub description: String,
}

/// Stable file identity (Windows volume-serial + file-index; Unix
/// device + inode). Mirrors `agenttidy_infrastructure::fs_probe::FileIdentity`
/// — duplicated here so `core` stays platform-agnostic (red line #11).
/// `agenttidy-infrastructure` is the only consumer that actually fills this
/// in; everywhere else it travels as opaque bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileIdentity {
    pub volume: u64,
    pub index: u64,
}

/// Snapshot of a unit's location at plan time. The revalidator compares
/// these four fields against freshly-observed values; any mismatch marks
/// the unit `Skipped`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceFingerprint {
    pub path: PathBuf,
    pub identity: FileIdentity,
    pub size_bytes: u64,
    pub mtime_ms: i64,
}

/// What action the executor will take. `Trash` ⇒ the unit is moved to the
/// OS trash as one whole. `ProviderOperation` ⇒ the provider's internal
/// operation (Phase 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupOperation {
    pub action: CleanupAction,
}

/// An atomic cleanup unit — what a provider proves can be operated on as
/// one whole (§9.5). Constructed by `AgentProviderAdapter::build_cleanup_units`.
///
/// `CleanupUnit` has no `ownership` field: only `Exclusive` resources can
/// form a unit (red line #6 + §9.5). The constructor enforces this; the
/// policy never sees Shared/Unknown candidates as units.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupUnit {
    pub id: String,
    pub provider: ProviderId,
    pub installation_id: String,
    pub kind: CleanupUnitKind,
    pub resources: Vec<ResourceRef>,
    pub estimated_reclaimable_bytes: u64,
    pub fingerprint: ResourceFingerprint,
    /// Where the artifact(s) live — the typed path set the executor
    /// operates on and the audit log records. `FileSet` carries *all*
    /// member paths (e.g. transcript + same-named side directory), not
    /// just the primary fingerprinted one.
    pub locator: CleanupLocator,
    pub preconditions: Vec<CleanupPrecondition>,
    pub operation: CleanupOperation,
    /// Resource kind this unit summarizes (informational for grouping +
    /// safety doc filtering). Today: `Workspace` for `kind: FileTree`.
    pub resource_kind: ResourceKind,
}

/// Decision the policy makes about a unit (Start.md §11). Pure: derived
/// only from the unit's static facts + the [`CleanupContext`] the
/// caller provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupDecision {
    pub risk: RiskLevel,
    pub recommended: bool,
    /// Short user-facing reasons — *what* makes this safe / unsafe.
    pub reasons: Vec<String>,
    /// Hard blockers — anything here forces `risk: Blocked`.
    pub blockers: Vec<String>,
}

/// One row in [`CleanupPlan::items`]. Carries the snapshot the executor
/// needs: unit id, fingerprint to compare against, action + risk. The
/// `installation_id` + `provider` are denormalized from the unit so the
/// backend can route `cleanup_revalidate` / `cleanup_execute` to the
/// right adapter without re-scanning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupItem {
    pub unit_id: String,
    pub installation_id: String,
    pub provider: ProviderId,
    pub fingerprint: ResourceFingerprint,
    /// Denormalized from the unit so the executor trashes / records the
    /// *whole* artifact without re-scanning (a `FileSet` item must move
    /// every member path, §9.5 atomicity).
    pub locator: CleanupLocator,
    /// Denormalized from the unit — the plan's honest reclaim estimate,
    /// also recorded as `reclaimed_bytes` on success (§16.3).
    pub estimated_reclaimable_bytes: u64,
    pub action: CleanupAction,
    pub risk: RiskLevel,
    pub reasons: Vec<String>,
    pub preconditions: Vec<CleanupPrecondition>,
}

/// Bucket totals — `Start.md` §6.1 wants byte totals per bucket, not
/// just item counts. Phase 6 ships all three pairs (count + bytes).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskSummary {
    pub low_risk: u32,
    pub low_risk_bytes: u64,
    pub review_required: u32,
    pub review_required_bytes: u64,
    pub blocked: u32,
    pub blocked_bytes: u64,
}

impl RiskSummary {
    pub fn add(&mut self, risk: RiskLevel, bytes: u64) {
        match risk {
            RiskLevel::LowRisk => {
                self.low_risk += 1;
                self.low_risk_bytes += bytes;
            }
            RiskLevel::ReviewRequired => {
                self.review_required += 1;
                self.review_required_bytes += bytes;
            }
            RiskLevel::Blocked => {
                self.blocked += 1;
                self.blocked_bytes += bytes;
            }
        }
    }
}

/// A plan is a snapshot of one cleanup intent. It expires after
/// `expires_at` (per §12.2). The frontend holds it between the two
/// confirmations; the backend never stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupPlan {
    pub id: String,
    /// Fingerprint of the originating `AgentSnapshot`'s metadata.
    pub scan_id: String,
    pub created_at: u64,
    /// `None` for "never expires" (provider-driven plans, future). All
    /// Phase 6 plans set this explicitly.
    pub expires_at: Option<u64>,
    pub items: Vec<CleanupItem>,
    pub total_reclaimable_bytes: u64,
    pub risk_summary: RiskSummary,
    /// SHA-256 over `(plan_id, scan_id, sorted_unit_ids, total_reclaimable_bytes, risk_summary)`.
    /// The executor refuses to run if the freshly-revalidated plan's hash
    /// differs from the frontend's.
    pub fingerprint: String,
}

/// Per-item result returned by `cleanup_execute`. Independent per item
/// (§12.3 — partial failure does not corrupt the report).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupItemOutcome {
    pub unit_id: String,
    pub outcome: CleanupOutcome,
    pub reclaimed_bytes: u64,
    /// Empty on `Removed`; populated on `Skipped` / `Failed`.
    pub reason: Option<String>,
}

/// Per-item result of `cleanup_revalidate`. The frontend uses this to
/// surface *what* invalidated each unit between the two confirmations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupRevalidationOutcome {
    pub unit_id: String,
    pub outcome: CleanupOutcome,
    /// Specific precondition kind that invalidated the unit (or `None`
    /// when the unit still passes).
    pub failed_precondition: Option<CleanupPreconditionKind>,
    pub reason: Option<String>,
}

/// Where the original artifact lived — mirrors `ResourceLocator` so a
/// future `DatabaseRecord` variant (§9.4) can be logged without string
/// rewriting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CleanupLocator {
    /// A single file at path.
    File { path: PathBuf },
    /// A directory treated as one unit (today's only kind).
    Dir { path: PathBuf },
    /// Multiple paths forming one resource (transcript + sidecars).
    FileSet { paths: Vec<PathBuf> },
    /// A provider DB row (Phase 7).
    DbRow {
        database: PathBuf,
        record_id: String,
    },
}

impl CleanupLocator {
    /// Primary path used by audit logs and revalidation comparisons.
    pub fn primary_path(&self) -> &PathBuf {
        match self {
            CleanupLocator::File { path } | CleanupLocator::Dir { path } => path,
            CleanupLocator::FileSet { paths } => paths.first().expect("non-empty file set"),
            CleanupLocator::DbRow { database, .. } => database,
        }
    }

    /// Every path this locator covers. The executor moves *all* members
    /// of a `FileSet` as one unit (§9.5 atomicity) and the revalidator
    /// checks each one still exists (§12.2 "path changed").
    pub fn paths(&self) -> Vec<&PathBuf> {
        match self {
            CleanupLocator::File { path } | CleanupLocator::Dir { path } => vec![path],
            CleanupLocator::FileSet { paths } => paths.iter().collect(),
            CleanupLocator::DbRow { database, .. } => vec![database],
        }
    }
}

/// One audit log entry — `Start.md` §16.3. Append-only JSONL at
/// `$HOME/.agenttidy/operations.jsonl` (Windows:
/// `%USERPROFILE%\.agenttidy\operations.jsonl`). No transcript bodies,
/// credentials, or file contents (the `path` is the original location
/// only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupEvent {
    /// `agenttidy.audit.v1` — schema version so consumers can detect
    /// breakage (§18.5).
    pub schema_version: String,
    pub platform: crate::installation::Platform,
    pub timestamp_ms: u64,
    pub plan_id: String,
    pub unit_id: String,
    pub provider: ProviderId,
    pub original_locator: CleanupLocator,
    pub action: CleanupAction,
    pub outcome: CleanupOutcome,
    pub reclaimed_bytes: u64,
    pub reason: Option<String>,
}

impl CleanupEvent {
    pub const SCHEMA_VERSION: &'static str = "agenttidy.audit.v1";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_level_serializes_as_kebab_case() {
        assert_eq!(
            serde_json::to_string(&RiskLevel::LowRisk).unwrap(),
            r#""low-risk""#
        );
        assert_eq!(
            serde_json::to_string(&RiskLevel::ReviewRequired).unwrap(),
            r#""review-required""#
        );
        assert_eq!(
            serde_json::to_string(&RiskLevel::Blocked).unwrap(),
            r#""blocked""#
        );
    }

    #[test]
    fn cleanup_outcome_roundtrips() {
        for outcome in [
            CleanupOutcome::Pending,
            CleanupOutcome::Removed,
            CleanupOutcome::Skipped,
            CleanupOutcome::Failed,
        ] {
            let json = serde_json::to_string(&outcome).unwrap();
            let back: CleanupOutcome = serde_json::from_str(&json).unwrap();
            assert_eq!(back, outcome);
        }
    }

    #[test]
    fn cleanup_unit_kind_serializes_as_kebab_case() {
        assert_eq!(
            serde_json::to_string(&CleanupUnitKind::FileTree).unwrap(),
            r#""file-tree""#
        );
        assert_eq!(
            serde_json::to_string(&CleanupUnitKind::FileSet).unwrap(),
            r#""file-set""#
        );
        assert_eq!(
            serde_json::to_string(&CleanupUnitKind::ProviderOperation).unwrap(),
            r#""provider-operation""#
        );
    }

    #[test]
    fn cleanup_action_kebab_case() {
        assert_eq!(
            serde_json::to_string(&CleanupAction::Trash).unwrap(),
            r#""trash""#
        );
        assert_eq!(
            serde_json::to_string(&CleanupAction::ProviderOperation).unwrap(),
            r#""provider-operation""#
        );
    }

    #[test]
    fn cleanup_locator_is_tagged() {
        let loc = CleanupLocator::Dir {
            path: PathBuf::from("/Users/x/.codex/Documents/Codex/task-a"),
        };
        let json = serde_json::to_string(&loc).unwrap();
        assert!(json.contains(r#""kind":"dir""#));
        let back: CleanupLocator = serde_json::from_str(&json).unwrap();
        assert_eq!(back, loc);
    }

    #[test]
    fn cleanup_plan_roundtrip_with_optional_expires() {
        let plan = CleanupPlan {
            id: "00000000000003e80000000000000001".into(),
            scan_id: "scan-fingerprint".into(),
            created_at: 1_700_000_000_000,
            expires_at: Some(1_700_000_300_000),
            items: vec![],
            total_reclaimable_bytes: 0,
            risk_summary: RiskSummary::default(),
            fingerprint: "abc123".into(),
        };
        let json = serde_json::to_string(&plan).unwrap();
        let back: CleanupPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, plan);
        let no_expiry = CleanupPlan {
            expires_at: None,
            ..plan.clone()
        };
        let json2 = serde_json::to_string(&no_expiry).unwrap();
        assert!(json2.contains("\"expires_at\":null"));
        let back2: CleanupPlan = serde_json::from_str(&json2).unwrap();
        assert_eq!(back2.expires_at, None);
    }

    #[test]
    fn risk_summary_add_buckets_by_risk() {
        let mut s = RiskSummary::default();
        s.add(RiskLevel::LowRisk, 100);
        s.add(RiskLevel::LowRisk, 50);
        s.add(RiskLevel::ReviewRequired, 200);
        s.add(RiskLevel::Blocked, 400);
        assert_eq!(s.low_risk, 2);
        assert_eq!(s.low_risk_bytes, 150);
        assert_eq!(s.review_required, 1);
        assert_eq!(s.review_required_bytes, 200);
        assert_eq!(s.blocked, 1);
        assert_eq!(s.blocked_bytes, 400);
    }

    #[test]
    fn cleanup_event_includes_schema_version_marker() {
        let event = CleanupEvent {
            schema_version: CleanupEvent::SCHEMA_VERSION.into(),
            platform: crate::installation::Platform::Macos,
            timestamp_ms: 1_700_000_000_000,
            plan_id: "plan-1".into(),
            unit_id: "unit-1".into(),
            provider: ProviderId::new(ProviderId::CODEX),
            original_locator: CleanupLocator::Dir {
                path: PathBuf::from("/x"),
            },
            action: CleanupAction::Trash,
            outcome: CleanupOutcome::Removed,
            reclaimed_bytes: 42,
            reason: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""schema_version":"agenttidy.audit.v1""#));
        let back: CleanupEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn cleanup_locator_primary_path_for_each_variant() {
        let file = CleanupLocator::File {
            path: PathBuf::from("/a/b.txt"),
        };
        assert_eq!(file.primary_path(), &PathBuf::from("/a/b.txt"));
        let dir = CleanupLocator::Dir {
            path: PathBuf::from("/a/dir"),
        };
        assert_eq!(dir.primary_path(), &PathBuf::from("/a/dir"));
        let set = CleanupLocator::FileSet {
            paths: vec![PathBuf::from("/a/1"), PathBuf::from("/a/2")],
        };
        assert_eq!(set.primary_path(), &PathBuf::from("/a/1"));
        let row = CleanupLocator::DbRow {
            database: PathBuf::from("/a/db.sqlite"),
            record_id: "thr-1".into(),
        };
        assert_eq!(row.primary_path(), &PathBuf::from("/a/db.sqlite"));
    }

    #[test]
    fn precondition_kinds_are_kebab() {
        let kinds = [
            (CleanupPreconditionKind::PathExists, "path-exists"),
            (CleanupPreconditionKind::PathUnchanged, "path-unchanged"),
            (
                CleanupPreconditionKind::IdentityUnchanged,
                "identity-unchanged",
            ),
            (CleanupPreconditionKind::SizeUnchanged, "size-unchanged"),
            (CleanupPreconditionKind::MtimeUnchanged, "mtime-unchanged"),
            (CleanupPreconditionKind::NoAgentRunning, "no-agent-running"),
            (CleanupPreconditionKind::SessionInactive, "session-inactive"),
            (
                CleanupPreconditionKind::SessionStillExclusive,
                "session-still-exclusive",
            ),
            (CleanupPreconditionKind::SchemaUnchanged, "schema-unchanged"),
            (
                CleanupPreconditionKind::WriterLockAbsent,
                "writer-lock-absent",
            ),
            (
                CleanupPreconditionKind::FilesystemEligible,
                "filesystem-eligible",
            ),
            (
                CleanupPreconditionKind::NoSymlinkOutsideRoot,
                "no-symlink-outside-root",
            ),
        ];
        for (kind, expected) in kinds {
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{expected}\"")
            );
        }
        // 12 §12.2 triggers (the agent listed 11 — we keep both PathExists
        // and PathUnchanged because PathExists is for early failure,
        // PathUnchanged is the §12.2 fingerprint trigger).
        let unique: std::collections::HashSet<_> = kinds.iter().map(|(k, _)| *k).collect();
        assert_eq!(unique.len(), kinds.len());
    }
}
