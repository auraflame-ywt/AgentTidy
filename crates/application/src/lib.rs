//! Application API — the single stable entry point shared by the desktop GUI
//! and the CLI (design doc `Start.md` §13):
//!
//! - Detect / Scan / List
//! - Plan / Validate / Execute
//!
//! Phase 4 wires the fixed v0.1 provider set here. Neither the GUI nor the
//! CLI may bypass this layer and touch providers or the filesystem directly.

use agenttidy_antigravity::AntigravityAdapter;
use agenttidy_claude_code::ClaudeCodeAdapter;
use agenttidy_codex::CodexAdapter;
use agenttidy_core::{
    AgentCapabilities, AgentInstallation, AgentSnapshot, CleanupAction, CleanupDecision,
    CleanupEvent, CleanupEvidence, CleanupItem, CleanupItemOutcome, CleanupOutcome, CleanupPlan,
    CleanupPreconditionKind, CleanupRevalidationOutcome, CleanupUnit, CleanupUnitKind, Ownership,
    ProviderId, ProviderRegistry, Resource, ResourceKind, RiskLevel, RiskSummary, ScanOptions,
    Session, SessionLifecycle,
};
use agenttidy_infrastructure::{
    audit_log,
    fs_probe::{walk_tree, EntryKind},
    operation_guard,
    paths::safe_canonicalize,
    trash::{self, PathTrashResult, TrashError, TrashOutcome},
};
use agenttidy_pi::PiAdapter;
use agenttidy_provider_api::{AgentProviderAdapter, ProviderInspection};
use agenttidy_workbuddy::WorkBuddyAdapter;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static CLEANUP_PLAN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// §11.1 "archived + inactive > 30d" window for the single LowRisk path.
const THIRTY_DAYS_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// Pure inputs the policy reads. `now_ms` is injected so the policy
/// stays deterministic (testable without a system clock).
pub struct CleanupContext<'a> {
    pub snapshot: &'a AgentSnapshot,
    pub inspection: &'a ProviderInspection,
    pub now_ms: u64,
}

/// Inspection facts and capabilities for one detected installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub installation: AgentInstallation,
    pub inspection: ProviderInspection,
    pub capabilities: AgentCapabilities,
}

/// Read-only result of applying the workspace-cleanup admission gates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceCleanupCandidate {
    pub installation_id: String,
    pub path: String,
    pub session_id: Option<String>,
    pub logical_bytes: u64,
    pub eligible: bool,
    pub blockers: Vec<String>,
}

/// The single shared application API for AgentTidy's current read-only flow.
pub struct Application {
    providers: ProviderRegistry<Box<dyn AgentProviderAdapter>>,
}

impl Application {
    /// Construct the static v0.1 registry. Duplicate ids are a programmer
    /// error caught at startup rather than a dynamically recoverable state.
    pub fn new() -> Self {
        let mut providers: ProviderRegistry<Box<dyn AgentProviderAdapter>> =
            ProviderRegistry::empty();
        providers
            .register(ProviderId::new(ProviderId::CODEX), Box::new(CodexAdapter))
            .expect("Codex registers once");
        providers
            .register(
                ProviderId::new(ProviderId::CLAUDE_CODE),
                Box::new(ClaudeCodeAdapter),
            )
            .expect("Claude Code registers once");
        providers
            .register(
                ProviderId::new(ProviderId::WORKBUDDY),
                Box::new(WorkBuddyAdapter),
            )
            .expect("WorkBuddy registers once");
        providers
            .register(ProviderId::new(ProviderId::PI), Box::new(PiAdapter))
            .expect("Pi registers once");
        providers
            .register(
                ProviderId::new(ProviderId::ANTIGRAVITY),
                Box::new(AntigravityAdapter),
            )
            .expect("Antigravity registers once");
        Self { providers }
    }

    /// Detect every known installation in stable provider order.
    pub async fn detect(&self) -> anyhow::Result<Vec<AgentInstallation>> {
        let mut installations = Vec::new();
        for (_, provider) in self.providers.iter() {
            installations.extend(provider.detect().await?);
        }
        Ok(installations)
    }

    /// Run the pre-scan diagnostic flow for every detected installation.
    pub async fn doctor(&self) -> anyhow::Result<Vec<DoctorReport>> {
        let mut reports = Vec::new();
        for installation in self.detect().await? {
            let provider = self
                .providers
                .get(&installation.provider)
                .expect("detected installation has a registered provider");
            let inspection = provider.inspect(&installation).await?;
            let capabilities = provider.capabilities(&inspection).await?;
            reports.push(DoctorReport {
                installation,
                inspection,
                capabilities,
            });
        }
        Ok(reports)
    }

    /// Scan every available installation with common read-only options.
    pub async fn scan(&self, options: &ScanOptions) -> anyhow::Result<Vec<AgentSnapshot>> {
        let mut snapshots = Vec::new();
        for installation in self.detect().await? {
            if !installation.is_available() {
                continue;
            }
            let provider = self
                .providers
                .get(&installation.provider)
                .expect("detected installation has a registered provider");
            snapshots.push(provider.scan(&installation, options).await?);
        }
        Ok(snapshots)
    }

    /// Produce a non-executable workspace preview. This is intentionally
    /// conservative: it does not create a CleanupPlan or mutate any path.
    pub async fn workspace_cleanup_candidates(
        &self,
    ) -> anyhow::Result<Vec<WorkspaceCleanupCandidate>> {
        let snapshots = self.scan(&ScanOptions::default()).await?;
        Ok(Self::workspace_cleanup_candidates_from_snapshots(
            &snapshots,
        ))
    }

    /// Apply workspace admission rules to already-scanned facts. Keeping this
    /// pure makes the conservative policy independently testable and lets
    /// future UI surfaces render the exact same preview as the CLI.
    ///
    /// Phase 6: refactored to project from [`Self::cleanup_plan_from_snapshots`]
    /// so the workspace-cleanup preview is a *view* on the same plan the
    /// GUI's Review & Tidy tab renders — one source of truth, no drift.
    pub fn workspace_cleanup_candidates_from_snapshots(
        snapshots: &[AgentSnapshot],
    ) -> Vec<WorkspaceCleanupCandidate> {
        // Phase 6: providers all return empty `build_cleanup_units`, so the
        // typed plan contains zero items. The legacy workspace-cleanup
        // admission (filesystem_eligible, single-session-match, WorkBuddy
        // automation gate) is preserved as a *legacy projection* on the
        // workspace-kind resources in the snapshot, so the existing CLI /
        // app callers do not break while the framework is empty. Phase 7
        // deletes this function once the typed plan carries everything.
        let mut candidates = Vec::new();
        for snapshot in snapshots {
            for workspace in snapshot
                .resources
                .iter()
                .filter(|resource| resource.kind == ResourceKind::Workspace)
            {
                let path = match &workspace.locator {
                    agenttidy_core::ResourceLocator::Dir { path } => path.clone(),
                    _ => continue,
                };
                let mut blockers = Vec::new();
                if workspace
                    .metadata
                    .get("filesystem_eligible")
                    .and_then(serde_json::Value::as_bool)
                    != Some(true)
                {
                    blockers.push(
                        "workspace contains Git, links, unknown entries, or unreadable paths"
                            .into(),
                    );
                }
                let matches: Vec<_> = snapshot
                    .sessions
                    .iter()
                    .filter(|session| {
                        session
                            .project
                            .as_ref()
                            .and_then(|project| project.cwd.as_ref())
                            == Some(&path)
                    })
                    .collect();
                if matches.len() != 1 {
                    blockers.push(format!(
                        "requires exactly one session cwd match; found {}",
                        matches.len()
                    ));
                }
                // WorkBuddy has automations that can reference arbitrary
                // cwds. Since Phase 7 the adapter reads the frozen
                // `automations.cwds` contract and exposes the union on the
                // workspace resource as `automation_cwd_refs`: absent ⇒
                // not provable, block (fail-closed); present and matching
                // this cwd ⇒ an automation still targets it, block;
                // present without this cwd ⇒ provably unreferenced.
                if workspace.provider.as_str() == ProviderId::WORKBUDDY {
                    match workspace.metadata.get("automation_cwd_refs") {
                        None => blockers
                            .push("WorkBuddy automation references are not yet verified".into()),
                        Some(serde_json::Value::Array(refs)) => {
                            let referenced = refs
                                .iter()
                                .any(|value| value.as_str().is_some_and(|cwd| cwd == path));
                            if referenced {
                                blockers.push("WorkBuddy automation references this cwd".into());
                            }
                        }
                        Some(_) => {
                            blockers.push("WorkBuddy automation references are malformed".into())
                        }
                    }
                }
                candidates.push(WorkspaceCleanupCandidate {
                    installation_id: snapshot.installation.id.clone(),
                    path,
                    session_id: matches
                        .first()
                        .map(|session| session.id.as_str().to_owned()),
                    logical_bytes: workspace.size.logical_bytes,
                    eligible: blockers.is_empty(),
                    blockers,
                });
            }
        }
        candidates
    }

    // -----------------------------------------------------------------
    // Phase 6: typed cleanup framework (Start.md §11, §12, §16.3).
    //
    // - `CleanupContext` + `cleanup_policy_evaluate` are the pure policy
    //   (§11) — no I/O, deterministic given `now_ms`.
    // - `cleanup_plan_from_snapshots` builds the typed plan (§12) by
    //   asking each provider's `build_cleanup_units`, evaluating the
    //   policy per unit, and stamping a fingerprint.
    // - `cleanup_revalidate` runs the §12.2 invalidation checks
    //   (filesystem + provider) and reports which (if any) trigger fired.
    // - `cleanup_execute` revalidates again, executes `trash::move_to_trash`
    //   for each selected unit whose preconditions still hold, and
    //   appends one `CleanupEvent` per item to the audit log (§16.3).
    //
    // Phase 7 providers opt in per resource type behind docs/safety/ gates.
    // -----------------------------------------------------------------

    /// Pure policy (§11). Maps a unit + context to a typed decision.
    ///
    /// Rules (Phase 6 conservative defaults; Phase 7 refines for
    /// individual resource kinds):
    /// - `kind: ProviderOperation` ⇒ `ReviewRequired` (no automated path).
    /// - `inspection.agent_running == true` ⇒ `Blocked`.
    /// - Snapshot has any `ScanProblem::Error` ⇒ `Blocked`.
    /// - Otherwise ⇒ `ReviewRequired` (the user must review every unit
    ///   in Phase 6; promoting to `LowRisk` requires age + lifecycle
    ///   facts Phase 7 supplies per-resource-kind).
    pub fn cleanup_policy_evaluate(
        unit: &CleanupUnit,
        ctx: &CleanupContext<'_>,
    ) -> CleanupDecision {
        let mut decision = CleanupDecision {
            risk: RiskLevel::Blocked,
            recommended: false,
            reasons: Vec::new(),
            blockers: Vec::new(),
        };

        match unit.kind {
            CleanupUnitKind::FileTree | CleanupUnitKind::FileSet => {
                decision
                    .reasons
                    .push(format!("kind is {:?} (trash-first eligible)", unit.kind));
            }
            CleanupUnitKind::ProviderOperation => {
                decision.reasons.push(
                    "ProviderOperation: requires provider cooperation; user must review".into(),
                );
            }
        }

        if ctx.inspection.agent_running {
            decision
                .blockers
                .push("agent is currently running (§16.2: concurrent execution not safe)".into());
        } else {
            decision.reasons.push("agent is not running".into());
        }

        let has_error = ctx
            .snapshot
            .problems
            .values()
            .any(|p| matches!(p, agenttidy_core::ScanProblem::Error { .. }));
        if has_error {
            decision
                .blockers
                .push("scan reported an Error problem for this installation".into());
        } else {
            decision.reasons.push("no Error-level scan problems".into());
        }

        // Session linkage (§9.3 + §11): every risk rule is lifecycle-
        // based, so a unit must resolve to its owning session through the
        // scan snapshot. No linkage ⇒ fail closed (red line #6: "never act
        // on what we do not understand").
        let linked_resources: Vec<&Resource> = unit
            .resources
            .iter()
            .filter_map(|reference| {
                ctx.snapshot
                    .resources
                    .iter()
                    .find(|resource| resource.id == reference.id)
            })
            .collect();
        // §11.1 LowRisk needs *every* condition to hold; any unmet one
        // below flips this off. Starts pessimistic only when there is no
        // evidence at all.
        let mut low_risk_eligible = true;
        if linked_resources.is_empty() {
            decision.blockers.push(
                "unit does not resolve to a resource in the scan snapshot (§11 fail-closed)".into(),
            );
            low_risk_eligible = false;
        }
        for resource in &linked_resources {
            // §9.5: only Exclusive resources may form units — anything
            // else is a contract violation and blocks (red line #6).
            if resource.ownership != Ownership::Exclusive {
                decision.blockers.push(format!(
                    "resource {} is not exclusively owned (§9.5)",
                    resource.id.as_str()
                ));
                low_risk_eligible = false;
            }
            if !resource.dependencies.is_empty() {
                decision.blockers.push(format!(
                    "resource {} has {} dependency/dependencies (§7.3: shared/unknown deps are Blocked)",
                    resource.id.as_str(),
                    resource.dependencies.len()
                ));
                low_risk_eligible = false;
            }
        }
        let sessions: Vec<&Session> = linked_resources
            .iter()
            .filter_map(|resource| resource.session_id.as_ref())
            .filter_map(|session_id| {
                ctx.snapshot
                    .sessions
                    .iter()
                    .find(|session| session.id == *session_id)
            })
            .collect();
        if sessions.is_empty() && !linked_resources.is_empty() {
            decision
                .blockers
                .push("unit has no owning session in the scan snapshot (§11 fail-closed)".into());
            low_risk_eligible = false;
        }

        // §11.1 archive-aware + §11.2 retention rules (v0.1):
        // Active/Unknown ⇒ Blocked; Archived + inactive > 30d keeps
        // LowRisk eligibility; Archived recent / Inactive ⇒ ReviewRequired
        // — §11.2: age alone never promotes a non-archived session.
        for session in &sessions {
            match &session.lifecycle {
                SessionLifecycle::Active => {
                    decision
                        .blockers
                        .push(format!("session {} is Active (§11.1)", session.id.as_str()));
                    low_risk_eligible = false;
                }
                SessionLifecycle::Unknown => {
                    decision.blockers.push(format!(
                        "session {} lifecycle is Unknown (§11.2 fail-closed)",
                        session.id.as_str()
                    ));
                    low_risk_eligible = false;
                }
                SessionLifecycle::Inactive => {
                    decision.reasons.push(format!(
                        "session {} is Inactive (§11.2: review required — age alone is not Low Risk)",
                        session.id.as_str()
                    ));
                    low_risk_eligible = false;
                }
                SessionLifecycle::Archived { archived_at } => {
                    // Archive is a weighted signal (§3.3), never
                    // permission on its own (red line #5).
                    let last_activity = session.updated_at.or(*archived_at).or(session.created_at);
                    match last_activity {
                        Some(last) if ctx.now_ms.saturating_sub(last) > THIRTY_DAYS_MS => {
                            decision.reasons.push(format!(
                                "session {} is Archived and inactive > 30d (§11.1)",
                                session.id.as_str()
                            ));
                        }
                        Some(_) => {
                            low_risk_eligible = false;
                            decision.reasons.push(format!(
                                "session {} is Archived but recently active (§11.1 → Review Required)",
                                session.id.as_str()
                            ));
                        }
                        None => {
                            low_risk_eligible = false;
                            decision.reasons.push(format!(
                                "session {} is Archived with unknown last-activity (§11.1 → Review Required)",
                                session.id.as_str()
                            ));
                        }
                    }
                }
            }
        }

        // Recoverability (§7.1 + §16.4): only Trash actions are
        // OS-recoverable; a ProviderOperation is never LowRisk in v0.1.
        if unit.operation.action != CleanupAction::Trash {
            low_risk_eligible = false;
        }

        if !decision.blockers.is_empty() {
            decision.risk = RiskLevel::Blocked;
        } else if low_risk_eligible {
            decision.risk = RiskLevel::LowRisk;
            decision.recommended = true;
            decision.reasons.push(
                "all §7.1 conditions hold: archived + inactive > 30d + exclusive + recoverable"
                    .into(),
            );
        } else {
            decision.risk = RiskLevel::ReviewRequired;
            decision.recommended = false;
            decision
                .reasons
                .push("no hard blockers; user review required per Start.md §7.2".into());
        }

        decision
    }

    /// Build a typed cleanup plan from provider units, policy decisions and
    /// read-only metadata evidence (§12.1). Provider validation is mandatory
    /// before a unit crosses the preview/execute IPC boundary.
    pub async fn cleanup_plan_from_snapshots(
        &self,
        snapshots: &[AgentSnapshot],
    ) -> anyhow::Result<CleanupPlan> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Capture the exact inspection used by policy; evidence travels across
        // independent preview/execute IPC requests rather than an instance cache.
        let mut inspection_by_installation = BTreeMap::new();
        for snapshot in snapshots {
            if let Some(provider) = self.providers.get(&snapshot.installation.provider) {
                let inspection = provider.inspect(&snapshot.installation).await?;
                inspection_by_installation.insert(snapshot.installation.id.clone(), inspection);
            }
        }

        let mut items: Vec<CleanupItem> = Vec::new();
        let mut risk_summary = RiskSummary::default();
        let mut total_reclaimable_bytes: u64 = 0;

        for snapshot in snapshots {
            let installation_id = snapshot.installation.id.clone();
            let provider = match self.providers.get(&snapshot.installation.provider) {
                Some(p) => p,
                None => continue,
            };
            let units = provider.build_cleanup_units(snapshot).await?;
            let inspection = inspection_by_installation
                .get(&installation_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing installation inspection"))?;

            let ctx = CleanupContext {
                snapshot,
                inspection: &inspection,
                now_ms,
            };

            for unit in units {
                // A failed unit is visible as Blocked, not a global preview
                // failure that hides other installed providers (§6.3 / §7.3).
                let mut decision = Self::cleanup_policy_evaluate(&unit, &ctx);
                let preconditions = match provider
                    .validate_cleanup_unit(&unit, &snapshot.installation)
                    .await
                {
                    Ok(preconditions) => preconditions,
                    Err(error) => {
                        decision.risk = RiskLevel::Blocked;
                        decision.recommended = false;
                        decision
                            .blockers
                            .push(format!("provider safety validation failed: {error}"));
                        unit.preconditions.clone()
                    }
                };
                let evidence = match cleanup_evidence(&unit, &inspection, snapshot) {
                    Ok(evidence) => evidence,
                    Err(error) => {
                        decision.risk = RiskLevel::Blocked;
                        decision.recommended = false;
                        decision
                            .blockers
                            .push(format!("cannot prove complete member evidence: {error}"));
                        CleanupEvidence::default()
                    }
                };
                // Hard blockers are the user's actionable explanation, not an
                // innocuous “agent not running” message ahead of the real cause.
                let reasons = decision
                    .blockers
                    .into_iter()
                    .chain(decision.reasons)
                    .collect();
                if decision.risk != RiskLevel::Blocked {
                    total_reclaimable_bytes =
                        total_reclaimable_bytes.saturating_add(unit.estimated_reclaimable_bytes);
                }
                risk_summary.add(decision.risk, unit.estimated_reclaimable_bytes);
                items.push(CleanupItem {
                    evidence,
                    unit_id: unit.id.clone(),
                    installation_id: unit.installation_id.clone(),
                    provider: unit.provider.clone(),
                    fingerprint: unit.fingerprint.clone(),
                    locator: unit.locator.clone(),
                    estimated_reclaimable_bytes: unit.estimated_reclaimable_bytes,
                    action: unit.operation.action,
                    risk: decision.risk,
                    reasons,
                    preconditions,
                });
            }
        }

        let plan_id = format!(
            "{:016x}{:08x}",
            now_ms,
            CLEANUP_PLAN_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let scan_id = scan_id_for(snapshots);
        let expires_at = Some(now_ms + 5 * 60 * 1000); // 5 min TTL — re-scan
                                                       // before this elapses
                                                       // or the executor will
                                                       // mark every item stale.
        let fingerprint = plan_fingerprint(
            &plan_id,
            &scan_id,
            &items,
            total_reclaimable_bytes,
            &risk_summary,
            now_ms,
            expires_at,
        );

        Ok(CleanupPlan {
            id: plan_id,
            scan_id,
            created_at: now_ms,
            expires_at,
            items,
            total_reclaimable_bytes,
            risk_summary,
            fingerprint,
        })
    }

    /// Revalidate every plan item against the freshly-observed filesystem
    /// and provider inspection. Returns one outcome per item — the GUI
    /// surfaces which (if any) §12.2 trigger invalidated the unit between
    /// the two confirmations. Pure side effects (read-only `inspect()` +
    /// `walk_tree`); no trash, no audit log.
    pub async fn cleanup_revalidate(
        &self,
        plan: &CleanupPlan,
    ) -> anyhow::Result<Vec<CleanupRevalidationOutcome>> {
        // §12.2 TTL: an expired plan is uniformly stale — report it per
        // item instead of silently re-checking the filesystem.
        if plan_expired(plan, now_ms()) {
            return Ok(plan
                .items
                .iter()
                .map(|item| CleanupRevalidationOutcome {
                    unit_id: item.unit_id.clone(),
                    outcome: CleanupOutcome::Skipped,
                    failed_precondition: None,
                    reason: Some("plan expired (TTL) — rescan and rebuild the plan (§12.2)".into()),
                })
                .collect());
        }

        let installations = self.detect().await?;
        let mut outcomes = Vec::with_capacity(plan.items.len());
        for item in &plan.items {
            outcomes.push(self.revalidate_live_item(item, &installations).await);
        }
        Ok(outcomes)
    }

    /// Reconstruct authoritative units rather than trusting a plan returned
    /// over IPC (§12.2). Fresh scans re-prove lifecycle, ownership, membership
    /// and policy; provider validation re-proves its path/DB/lock contracts.
    async fn revalidate_live_item(
        &self,
        item: &CleanupItem,
        installations: &[AgentInstallation],
    ) -> CleanupRevalidationOutcome {
        let rejected = |kind, reason: String| CleanupRevalidationOutcome {
            unit_id: item.unit_id.clone(),
            outcome: CleanupOutcome::Skipped,
            failed_precondition: Some(kind),
            reason: Some(reason),
        };
        let Some(installation) = installations.iter().find(|installation| {
            installation.id == item.installation_id
                && installation.provider == item.provider
                && installation.is_available()
        }) else {
            return rejected(
                CleanupPreconditionKind::PathExists,
                "installation is missing, unavailable or belongs to another provider".into(),
            );
        };
        let Some(provider) = self.providers.get(&item.provider) else {
            return rejected(
                CleanupPreconditionKind::PathExists,
                "provider unavailable".into(),
            );
        };
        let inspection = match provider.inspect(installation).await {
            Ok(inspection) => inspection,
            Err(error) => {
                return rejected(
                    CleanupPreconditionKind::NoAgentRunning,
                    format!("cannot inspect installation safely: {error}"),
                )
            }
        };
        let filesystem = revalidate_item(item, Some(&inspection));
        if filesystem.outcome != CleanupOutcome::Pending {
            return filesystem;
        }
        if inspection.version != item.evidence.provider_version
            || inspection.schema_versions != item.evidence.schema_versions
            || inspection.journal_modes != item.evidence.journal_modes
        {
            return rejected(
                CleanupPreconditionKind::SchemaUnchanged,
                "provider version, schema or journal mode changed — rescan".into(),
            );
        }
        let snapshot = match provider.scan(installation, &ScanOptions::default()).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return rejected(
                    CleanupPreconditionKind::SessionInactive,
                    format!("cannot re-prove session state: {error}"),
                )
            }
        };
        let units = match provider.build_cleanup_units(&snapshot).await {
            Ok(units) => units,
            Err(error) => {
                return rejected(
                    CleanupPreconditionKind::SessionInactive,
                    format!("cannot rebuild cleanup units: {error}"),
                )
            }
        };
        let Some(unit) = units.iter().find(|unit| unit.id == item.unit_id) else {
            return rejected(
                CleanupPreconditionKind::SessionInactive,
                "session no longer forms a verified cleanup unit — rescan".into(),
            );
        };
        if unit.locator != item.locator
            || unit.operation.action != item.action
            || unit.provider != item.provider
            || unit.installation_id != item.installation_id
        {
            return rejected(
                CleanupPreconditionKind::PathUnchanged,
                "cleanup action or unit membership changed — rescan".into(),
            );
        }
        if let Err(error) = provider.validate_cleanup_unit(unit, installation).await {
            return rejected(
                CleanupPreconditionKind::FilesystemEligible,
                format!("provider safety validation failed: {error}"),
            );
        }
        let final_inspection = match provider.inspect(installation).await {
            Ok(inspection) => inspection,
            Err(error) => {
                return rejected(
                    CleanupPreconditionKind::NoAgentRunning,
                    format!("final inspection failed: {error}"),
                )
            }
        };
        if final_inspection != inspection {
            return rejected(
                CleanupPreconditionKind::NoAgentRunning,
                "provider inspection changed while revalidating — rescan".into(),
            );
        }
        if inspection.readable_roots.values().any(|readable| !readable)
            || inspection
                .problems
                .values()
                .any(|problem| matches!(problem, agenttidy_core::ScanProblem::Error { .. }))
        {
            return rejected(
                CleanupPreconditionKind::FilesystemEligible,
                "provider inspection cannot prove readable, safe state".into(),
            );
        }
        let ctx = CleanupContext {
            snapshot: &snapshot,
            inspection: &inspection,
            now_ms: now_ms(),
        };
        let decision = Self::cleanup_policy_evaluate(unit, &ctx);
        if item.risk == RiskLevel::Blocked
            || decision.risk == RiskLevel::Blocked
            || item.risk != decision.risk
        {
            return rejected(
                CleanupPreconditionKind::SessionInactive,
                format!(
                    "cleanup is blocked or its risk changed: {}",
                    decision.blockers.join("; ")
                ),
            );
        }
        let current = match cleanup_evidence(unit, &inspection, &snapshot) {
            Ok(evidence) => evidence,
            Err(error) => {
                return rejected(
                    CleanupPreconditionKind::NoSymlinkOutsideRoot,
                    format!("cannot re-prove all member paths: {error}"),
                )
            }
        };
        if current.canonical_paths != item.evidence.canonical_paths {
            return rejected(
                CleanupPreconditionKind::PathUnchanged,
                "canonical member paths changed — rescan".into(),
            );
        }
        if current.members != item.evidence.members {
            return rejected(
                CleanupPreconditionKind::IdentityUnchanged,
                "file-set member identity, size, mtime or membership changed — rescan".into(),
            );
        }
        filesystem
    }

    /// Execute the selected items: revalidate, then call
    /// `trash::move_to_trash` for each item whose preconditions still
    /// hold. Writes one `CleanupEvent` per item to the audit log.
    /// Idempotent at the unit level (AlreadyGone ⇒ Skipped, not Removed).
    ///
    /// `fingerprint` is the confirmation guard: if it doesn't match
    /// `plan.fingerprint` exactly, no item is touched and an error is
    /// returned (defends against the user confirming a stale plan).
    pub async fn cleanup_execute(
        &self,
        plan: &CleanupPlan,
        fingerprint: &str,
        selected_unit_ids: &[String],
    ) -> anyhow::Result<Vec<CleanupItemOutcome>> {
        if fingerprint != plan.fingerprint {
            anyhow::bail!(
                "plan fingerprint mismatch: expected {}, got {}",
                plan.fingerprint,
                fingerprint
            );
        }

        // §12.2 TTL (fail-closed): a stale plan is refused outright —
        // never silently executed from outdated facts. Enforced here, not
        // only in the GUI, so raw IPC callers cannot bypass it.
        if plan_expired(plan, now_ms()) {
            anyhow::bail!(
                "plan expired (TTL) — rescan and rebuild the plan before executing (§12.2)"
            );
        }

        let expected = plan_fingerprint(
            &plan.id,
            &plan.scan_id,
            &plan.items,
            plan.total_reclaimable_bytes,
            &plan.risk_summary,
            plan.created_at,
            plan.expires_at,
        );
        if expected != plan.fingerprint {
            anyhow::bail!("plan payload fingerprint mismatch — rescan before executing");
        }
        let ids: std::collections::BTreeSet<_> =
            plan.items.iter().map(|item| &item.unit_id).collect();
        if ids.len() != plan.items.len() || selected_unit_ids.iter().any(|id| !ids.contains(id)) {
            anyhow::bail!("invalid or duplicate cleanup unit ids");
        }
        // A native DB mutation invalidates the shared DB/WAL evidence of every
        // other row in this plan. Until providers expose a coordinated batch
        // contract, reject the whole selection before locks/audit/mutation;
        // never silently execute only its first row (§12.2 / §12.3).
        validate_database_batch(plan, selected_unit_ids)?;
        // Serialize before inspecting, not after it. A previous operation may
        // change a shared DB; each item is checked again under this guard.
        let _guard = operation_guard::OperationGuard::acquire()
            .map_err(|error| anyhow::anyhow!("cannot acquire cleanup operation guard: {error}"))?;
        // If the audit destination is unavailable, fail before touching any
        // selected resource. Results cannot be made durable after the fact.
        let mut audit = audit_log::AuditWriter::open()?;
        let installations = self.detect().await?;
        let mut audit_failure: Option<String> = None;
        let mut results = Vec::new();
        for item in &plan.items {
            if !selected_unit_ids.contains(&item.unit_id) {
                // User did not select this unit; record as Pending skipped.
                results.push(CleanupItemOutcome {
                    unit_id: item.unit_id.clone(),
                    outcome: CleanupOutcome::Skipped,
                    reclaimed_bytes: 0,
                    reason: Some("user did not select this unit".into()),
                });
                continue;
            }
            if let Some(error) = &audit_failure {
                results.push(CleanupItemOutcome {
                    unit_id: item.unit_id.clone(),
                    outcome: CleanupOutcome::Skipped,
                    reclaimed_bytes: 0,
                    reason: Some(format!(
                        "batch stopped because audit logging failed: {error}"
                    )),
                });
                continue;
            }
            let revalidation = if plan_expired(plan, now_ms()) {
                CleanupRevalidationOutcome {
                    unit_id: item.unit_id.clone(),
                    outcome: CleanupOutcome::Skipped,
                    failed_precondition: None,
                    reason: Some("plan expired during execution — rescan".into()),
                }
            } else {
                self.revalidate_live_item(item, &installations).await
            };

            let locator = item.locator.clone();
            let action = item.action;

            if revalidation.outcome == CleanupOutcome::Pending && !plan_expired(plan, now_ms()) {
                let intent = CleanupEvent {
                    schema_version: CleanupEvent::SCHEMA_VERSION.into(),
                    platform: agenttidy_infrastructure::paths::host_platform(),
                    timestamp_ms: now_ms(),
                    plan_id: plan.id.clone(),
                    unit_id: item.unit_id.clone(),
                    provider: item.provider.clone(),
                    original_locator: locator.clone(),
                    action,
                    outcome: CleanupOutcome::Pending,
                    reclaimed_bytes: 0,
                    reason: Some("execution intent; final result not recorded yet".into()),
                };
                if let Err(error) = audit.record(&intent) {
                    audit_failure = Some(error.to_string());
                    results.push(CleanupItemOutcome {
                        unit_id: item.unit_id.clone(),
                        outcome: CleanupOutcome::Failed,
                        reclaimed_bytes: 0,
                        reason: Some(format!("audit intent failed before mutation: {error}")),
                    });
                    continue;
                }
            }

            // Execute: each action type has its own executor.
            //
            // - `Trash` is the OS-recycle flow that always ran here —
            //   `move_paths_to_trash` moves every member of a `FileSet`
            //   as one unit (§9.5); per-path results aggregate into one
            //   honest item outcome (§12.3).
            // - `ProviderOperation` is dispatched to the provider's
            //   `execute_provider_operation` trait method (Phase 7 —
            //   `docs/safety/provider-operation.md`, `Start.md` §12.4).
            //   Providers that have not opted in keep the legacy
            //   `Failed("not supported")` default. Provider-specific
            //   protocol/version gates live inside the provider, not here —
            //   Application stays policy-only, no per-provider logic
            //   (red line #2).
            let (outcome, reclaimed_bytes, reason) =
                if revalidation.outcome != CleanupOutcome::Pending {
                    (revalidation.outcome, 0, revalidation.reason)
                } else if plan_expired(plan, now_ms()) {
                    (
                        CleanupOutcome::Skipped,
                        0,
                        Some("plan expired while revalidating — rescan".into()),
                    )
                } else {
                    match action {
                        CleanupAction::Trash => {
                            let paths: Vec<PathBuf> =
                                item.locator.paths().into_iter().cloned().collect();
                            aggregate_trash_results(&trash::move_paths_to_trash(&paths))
                        }
                        CleanupAction::ProviderOperation => {
                            let provider = self.providers.get(&item.provider).ok_or_else(|| {
                                anyhow::anyhow!(
                                    "no registered adapter for provider {:?}",
                                    item.provider
                                )
                            })?;
                            // Resolve fresh detection facts in this request. No state
                            // from cleanup_preview is needed by this Application instance.
                            let installation = installations
                                .iter()
                                .find(|installation| {
                                    installation.id == item.installation_id
                                        && installation.provider == item.provider
                                })
                                .ok_or_else(|| anyhow::anyhow!("installation vanished"))?;
                            match provider
                                .execute_provider_operation(item, installation)
                                .await
                            {
                                Ok(outcome) => {
                                    (outcome.outcome, outcome.reclaimed_bytes, outcome.reason)
                                }
                                Err(error) => (
                                    CleanupOutcome::Failed,
                                    0,
                                    Some(format!("provider operation failed: {error}")),
                                ),
                            }
                        }
                    }
                };

            // The synced intent remains available after a crash. A result
            // write failure stops the batch; never hide a completed mutation.
            let event = CleanupEvent {
                schema_version: CleanupEvent::SCHEMA_VERSION.into(),
                platform: agenttidy_infrastructure::paths::host_platform(),
                timestamp_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                plan_id: plan.id.clone(),
                unit_id: item.unit_id.clone(),
                provider: item.provider.clone(),
                original_locator: locator,
                action,
                outcome,
                reclaimed_bytes,
                reason: reason.clone(),
            };
            if let Err(error) = audit.record(&event) {
                audit_failure = Some(error.to_string());
                results.push(CleanupItemOutcome {
                    unit_id: item.unit_id.clone(),
                    outcome: CleanupOutcome::Failed,
                    reclaimed_bytes: 0,
                    reason: Some(format!("operation outcome was {outcome:?}; audit result write failed: {error}; batch stopped")),
                });
                continue;
            }
            results.push(CleanupItemOutcome {
                unit_id: item.unit_id.clone(),
                outcome,
                reclaimed_bytes,
                reason,
            });
        }
        Ok(results)
    }
}

/// Admit at most one selected operation per physical database (§12.2).
/// Resolve live paths rather than trusting caller-supplied canonical evidence;
/// aliases must not circumvent the single-database-operation restriction.
fn validate_database_batch(plan: &CleanupPlan, selected: &[String]) -> anyhow::Result<()> {
    let mut databases = std::collections::BTreeSet::new();
    for item in &plan.items {
        if !selected.contains(&item.unit_id) {
            continue;
        }
        // A coordinated FileSet may contain a shared DB/cache as its primary
        // state object too. Serialize every provider operation on that identity.
        if item.action == CleanupAction::ProviderOperation {
            let Some(database) = item.locator.paths().first().copied() else {
                anyhow::bail!("empty provider operation locator");
            };
            let canonical = safe_canonicalize(database)?;
            let identity = agenttidy_infrastructure::fs_probe::file_identity(&canonical)
                .ok_or_else(|| anyhow::anyhow!("cannot identify selected database"))?;
            anyhow::ensure!(databases.insert((identity.volume, identity.index)),
                "multiple selected operations share a database; select one database session per fresh preview until coordinated database batches are supported (§12.2)");
        }
    }
    Ok(())
}

/// Capture metadata for every member without following links (§12.2 / §16.1).
/// A directory's children are part of the reviewed unit, not an untracked
/// attachment that can change between the two confirmations.
fn cleanup_evidence(
    unit: &CleanupUnit,
    inspection: &ProviderInspection,
    snapshot: &AgentSnapshot,
) -> anyhow::Result<CleanupEvidence> {
    let mut evidence = CleanupEvidence {
        provider_version: inspection.version.clone(),
        schema_versions: inspection.schema_versions.clone(),
        journal_modes: inspection.journal_modes.clone(),
        ..CleanupEvidence::default()
    };
    let mut roots: Vec<PathBuf> = unit.locator.paths().into_iter().cloned().collect();
    // A DB operation still belongs to a transcript resource. Retain its
    // metadata too so a rollout-only update cannot escape revalidation.
    for reference in &unit.resources {
        let resource = snapshot
            .resources
            .iter()
            .find(|resource| resource.id == reference.id)
            .ok_or_else(|| anyhow::anyhow!("cleanup resource disappeared"))?;
        match &resource.locator {
            agenttidy_core::ResourceLocator::File { path }
            | agenttidy_core::ResourceLocator::Dir { path } => roots.push(PathBuf::from(path)),
            agenttidy_core::ResourceLocator::FileSet { paths } => {
                roots.extend(paths.iter().map(PathBuf::from))
            }
            agenttidy_core::ResourceLocator::DatabaseRecord { database, .. } => {
                roots.push(PathBuf::from(database))
            }
        }
    }
    roots.sort();
    roots.dedup();
    // WAL changes do not necessarily change the main database's mtime.
    // Include existing SQLite sidecars and their membership in the evidence.
    if let agenttidy_core::CleanupLocator::DbRow { database, .. } = &unit.locator {
        for suffix in ["-wal", "-shm"] {
            let mut path = database.as_os_str().to_os_string();
            path.push(suffix);
            let path = PathBuf::from(path);
            if path.symlink_metadata().is_ok() {
                roots.push(path);
            }
        }
    }
    anyhow::ensure!(!roots.is_empty(), "empty cleanup locator");
    let mut members = BTreeMap::new();
    for root in roots {
        let walked = walk_tree(&root);
        anyhow::ensure!(
            walked.problems.is_empty() && !walked.entries.is_empty(),
            "cannot inspect complete unit at {}",
            root.display()
        );
        for entry in walked.entries {
            anyhow::ensure!(
                matches!(entry.kind, EntryKind::File | EntryKind::Dir),
                "link or unknown entry in cleanup unit: {}",
                entry.path.display()
            );
            let canonical = safe_canonicalize(&entry.path)?;
            evidence
                .canonical_paths
                .insert(entry.path.clone(), canonical);
            let identity = entry
                .identity
                .or_else(|| agenttidy_infrastructure::fs_probe::file_identity(&entry.path))
                .ok_or_else(|| anyhow::anyhow!("cannot fingerprint {}", entry.path.display()))?;
            let modified = std::fs::symlink_metadata(&entry.path)?
                .modified()?
                .duration_since(UNIX_EPOCH)?
                .as_millis() as i64;
            members.insert(
                entry.path.clone(),
                agenttidy_core::ResourceFingerprint {
                    path: entry.path,
                    identity: agenttidy_core::FileIdentity {
                        volume: identity.volume,
                        index: identity.index,
                    },
                    size_bytes: entry.logical_len,
                    mtime_ms: modified,
                },
            );
        }
    }
    evidence.members = members.into_values().collect();
    Ok(evidence)
}

/// §12.2 plan TTL: once `expires_at` passes the plan is uniformly stale
/// and the user must rescan (`None` = never expires).
fn plan_expired(plan: &CleanupPlan, now_ms: u64) -> bool {
    plan.expires_at
        .is_some_and(|expires_at| now_ms > expires_at)
}

/// Current wall-clock time in epoch ms. Wall-clock (not monotonic) is
/// correct here: `CleanupPlan.expires_at` is a wall-clock timestamp that
/// round-trips through the IPC boundary.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Hash the canonical plan tuple so the executor can reject a stale
/// confirmation. `sha2::Sha256` is the only thing we need from the
/// `sha2` workspace dep; no native deps, no MSRV gymnastics.
fn plan_fingerprint(
    plan_id: &str,
    scan_id: &str,
    items: &[CleanupItem],
    total_reclaimable_bytes: u64,
    risk_summary: &RiskSummary,
    created_at: u64,
    expires_at: Option<u64>,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let payload = (
        plan_id,
        scan_id,
        items,
        total_reclaimable_bytes,
        risk_summary,
        created_at,
        expires_at,
    );
    // Domain structs have infallible JSON serialization; never hash a
    // fallback empty string that could make distinct plans look identical.
    hasher.update(serde_json::to_vec(&payload).expect("cleanup plan serializes"));
    let digest = hasher.finalize();
    format!("{:x}", digest)
}

/// Stable hash of a snapshot set — used as `CleanupPlan.scan_id` so the
/// revalidator can correlate a plan back to the snapshot it was built
/// from. SHA-256 again (Start.md §12 line 689).
fn scan_id_for(snapshots: &[AgentSnapshot]) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(snapshots).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    format!("scan-{:x}", digest)
}

/// Aggregate per-path trash results into one §12.3 item outcome.
/// Pure, so the partial-failure rules are unit-testable without a real
/// trash service (which is environment-dependent — see `trash.rs`).
/// A Trash move is not physical disk reclamation; actual freed bytes stay
/// zero. `AlreadyGone` / `NotFound` are idempotent skips, never fresh success.
///
/// Lock classification (`docs/safety/session-transcript-fileset.md`
/// Concurrency): members held open by another process report `Locked`.
/// A **locked-only** failure with nothing moved is transient — the
/// item is `Skipped` with a retry hint rather than `Failed`. Any
/// non-locked failure, or a lock that arrives *after* some members
/// already moved, keeps the honest `Failed`: a partial FileSet is not
/// the measured unit and must never look like a clean skip.
fn aggregate_trash_results(results: &[PathTrashResult]) -> (CleanupOutcome, u64, Option<String>) {
    let mut removed = 0usize;
    let mut locked: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for (path, result) in results {
        match result {
            Ok(TrashOutcome::Removed) => removed += 1,
            Ok(TrashOutcome::AlreadyGone) | Err(TrashError::NotFound(_)) => {}
            Ok(TrashOutcome::Locked) => locked.push(path.display().to_string()),
            Ok(TrashOutcome::PlatformError(message)) => {
                failures.push(format!("{}: {message}", path.display()));
            }
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    if !failures.is_empty() {
        // §12.3: partial movement must be reported honestly — members
        // that did move are recoverable from the OS trash, the rest were
        // not touched. `reclaimed_bytes` stays 0: a partial FileSet is
        // not the measured unit.
        let partial = if removed > 0 {
            format!(
                "{removed}/{} member(s) moved to trash before failure; ",
                results.len()
            )
        } else {
            String::new()
        };
        return (
            CleanupOutcome::Failed,
            0,
            Some(format!("{partial}{}", failures.join("; "))),
        );
    }
    if !locked.is_empty() {
        if removed > 0 {
            // Some members moved, the rest are locked — the unit is
            // partially moved, not cleanly skipped (§12.3 honesty).
            return (
                CleanupOutcome::Failed,
                0,
                Some(format!(
                    "{removed}/{} member(s) moved to trash; the rest are locked by \
                     another process: {}",
                    results.len(),
                    locked.join("; ")
                )),
            );
        }
        // Locked-only, nothing moved: transient, retryable.
        return (
            CleanupOutcome::Skipped,
            0,
            Some(format!(
                "file locked by another process: {} — close the holder and retry",
                locked.join("; ")
            )),
        );
    }
    if removed > 0 {
        (CleanupOutcome::Removed, 0,
            Some("moved to system Trash; this does not free disk space until Trash is emptied manually".into()))
    } else {
        (
            CleanupOutcome::Skipped,
            0,
            Some("all unit members already gone (idempotent skip, §12.3)".into()),
        )
    }
}

/// Run the §12.2 invalidation checks against one plan item.
///
/// Walks the filesystem for the item's path and compares against its
/// `ResourceFingerprint`. Then walks each declared precondition and
/// surfaces the *first* failed one (the GUI uses this for the per-row
/// "stale" badge). Returns `CleanupOutcome::Pending` + reason `None`
/// when every check passes.
fn revalidate_item(
    item: &CleanupItem,
    inspection: Option<&ProviderInspection>,
) -> CleanupRevalidationOutcome {
    // 1. Inspection-level checks.
    if let Some(inspection) = inspection {
        if inspection.agent_running {
            return CleanupRevalidationOutcome {
                unit_id: item.unit_id.clone(),
                outcome: CleanupOutcome::Skipped,
                failed_precondition: Some(CleanupPreconditionKind::NoAgentRunning),
                reason: Some("agent is running (§16.2)".into()),
            };
        }
    }

    // 2. Unit membership (§9.5 + §12.2 "path changed"): every locator
    // member must still exist — a `FileSet` whose side directory was
    // removed out-of-band is no longer the unit that was planned.
    for path in item.locator.paths() {
        if path.symlink_metadata().is_err() {
            return CleanupRevalidationOutcome {
                unit_id: item.unit_id.clone(),
                outcome: CleanupOutcome::Skipped,
                failed_precondition: Some(CleanupPreconditionKind::PathExists),
                reason: Some(format!("unit member no longer exists: {}", path.display())),
            };
        }
    }

    // 3. Filesystem-level checks: walk_tree the path, compare the root
    // entry against the recorded fingerprint.
    let outcome = walk_tree(&item.fingerprint.path);
    if outcome.problems.iter().any(|p| {
        matches!(
            p,
            agenttidy_infrastructure::fs_probe::WalkProblem::Unreadable { .. }
        )
    }) {
        return CleanupRevalidationOutcome {
            unit_id: item.unit_id.clone(),
            outcome: CleanupOutcome::Skipped,
            failed_precondition: Some(CleanupPreconditionKind::PathExists),
            reason: Some("path became unreadable between plan and execute".into()),
        };
    }

    let root_entry = outcome
        .entries
        .iter()
        .find(|e| e.path == item.fingerprint.path);
    let entry = match root_entry {
        Some(e) => e,
        None => {
            return CleanupRevalidationOutcome {
                unit_id: item.unit_id.clone(),
                outcome: CleanupOutcome::Skipped,
                failed_precondition: Some(CleanupPreconditionKind::PathExists),
                reason: Some("path no longer exists at the recorded location".into()),
            };
        }
    };

    // Size comparison (we only check files; for directories the size
    // comparison is skipped — dir sizes fluctuate as files are added).
    if matches!(entry.kind, EntryKind::File) && entry.logical_len != item.fingerprint.size_bytes {
        return CleanupRevalidationOutcome {
            unit_id: item.unit_id.clone(),
            outcome: CleanupOutcome::Skipped,
            failed_precondition: Some(CleanupPreconditionKind::SizeUnchanged),
            reason: Some(format!(
                "size changed: was {}, now {}",
                item.fingerprint.size_bytes, entry.logical_len
            )),
        };
    }

    // mtime comparison (Unix mtime is i64; ResourceFingerprint carries i64).
    if let Ok(metadata) = std::fs::symlink_metadata(&item.fingerprint.path) {
        if let Ok(modified) = metadata.modified() {
            if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                let current_mtime_ms = duration.as_millis() as i64;
                if (current_mtime_ms - item.fingerprint.mtime_ms).abs() > 1000 {
                    return CleanupRevalidationOutcome {
                        unit_id: item.unit_id.clone(),
                        outcome: CleanupOutcome::Skipped,
                        failed_precondition: Some(CleanupPreconditionKind::MtimeUnchanged),
                        reason: Some(format!(
                            "mtime changed: was {}, now {}",
                            item.fingerprint.mtime_ms, current_mtime_ms
                        )),
                    };
                }
            }
        }
    }

    // Identity comparison (volume + index) — only when both sides have it.
    if let Some(current_identity) = entry.identity {
        let recorded_volume = item.fingerprint.identity.volume;
        let recorded_index = item.fingerprint.identity.index;
        if current_identity.volume != recorded_volume || current_identity.index != recorded_index {
            return CleanupRevalidationOutcome {
                unit_id: item.unit_id.clone(),
                outcome: CleanupOutcome::Skipped,
                failed_precondition: Some(CleanupPreconditionKind::IdentityUnchanged),
                reason: Some("file identity (volume/index) changed".into()),
            };
        }
    }

    CleanupRevalidationOutcome {
        unit_id: item.unit_id.clone(),
        outcome: CleanupOutcome::Pending,
        failed_precondition: None,
        reason: None,
    }
}

/// Empty inspection for deterministic unit tests, never a production
/// substitute for missing safety facts (§12.2 fail-closed).
#[cfg(test)]
trait ProviderInspectionExt {
    fn empty_for_testing() -> Self;
}

#[cfg(test)]
impl ProviderInspectionExt for ProviderInspection {
    fn empty_for_testing() -> Self {
        Self {
            version: None,
            schema_versions: BTreeMap::new(),
            journal_modes: BTreeMap::new(),
            agent_running: false,
            readable_roots: BTreeMap::new(),
            unknown_structures: Vec::new(),
            problems: BTreeMap::new(),
        }
    }
}

impl Default for Application {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agenttidy_core::{
        AgentInstallation, ManagedBy, Ownership, Platform, ProjectRef, Resource, ResourceId,
        ResourceLocator, ResourceRef, Session, SessionId, SessionLifecycle, SizeInfo,
    };
    use std::collections::BTreeMap;

    /// Serializes tests that mutate the process-global HOME / PATH
    /// env vars *and* acquire the cross-process operation guard.
    /// `cleanup_execute` resolves both the guard path and the audit
    /// log path from `$HOME`, so two concurrent execute-tests stomp
    /// each other's env and the guard then sees its own pid in a
    /// leftover lock file ("another cleanup operation is in
    /// progress"). Tests that don't touch env or the guard don't
    /// need this lock. Async-aware (`tokio::sync::Mutex`) because
    /// the guarded test bodies hold it across `.await` points.
    static EXECUTE_ENV_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Locked-only trash failures aggregate to `Skipped` with a retry
    /// hint; a lock arriving after other members already moved stays
    /// `Failed` (a partially-moved FileSet is never a clean skip).
    /// `docs/safety/session-transcript-fileset.md` Concurrency.
    #[test]
    fn aggregate_locked_paths_yield_skipped_outcome() {
        use agenttidy_infrastructure::trash::TrashOutcome;
        use std::path::PathBuf;

        let locked_only = vec![
            (PathBuf::from("/tmp/a.jsonl"), Ok(TrashOutcome::Locked)),
            (PathBuf::from("/tmp/a"), Ok(TrashOutcome::Locked)),
        ];
        let (outcome, bytes, reason) = aggregate_trash_results(&locked_only);
        assert_eq!(outcome, CleanupOutcome::Skipped);
        assert_eq!(bytes, 0);
        let reason = reason.expect("locked skip carries a reason");
        assert!(
            reason.contains("file locked by another process"),
            "unexpected reason: {reason}"
        );

        // Mixed: one member moved, the other locked — honest partial
        // failure, not a skip.
        let partial = vec![
            (PathBuf::from("/tmp/b.jsonl"), Ok(TrashOutcome::Removed)),
            (PathBuf::from("/tmp/b"), Ok(TrashOutcome::Locked)),
        ];
        let (outcome, bytes, reason) = aggregate_trash_results(&partial);
        assert_eq!(outcome, CleanupOutcome::Failed);
        assert_eq!(bytes, 0);
        assert!(
            reason
                .unwrap_or_default()
                .contains("locked by another process"),
            "mixed outcome must name the locked member"
        );
    }

    #[test]
    fn static_registry_contains_all_verified_provider_adapters() {
        let app = Application::new();
        assert_eq!(app.providers.len(), 5);
        for id in [
            ProviderId::CLAUDE_CODE,
            ProviderId::CODEX,
            ProviderId::WORKBUDDY,
            ProviderId::ANTIGRAVITY,
            ProviderId::PI,
        ] {
            assert!(app.providers.get(&ProviderId::new(id)).is_some());
        }
    }

    fn snapshot_with_workspace(
        provider: &str,
        workspace_path: &str,
        session_cwds: &[&str],
        filesystem_eligible: bool,
    ) -> AgentSnapshot {
        let provider_id = ProviderId::new(provider);
        let installation = AgentInstallation {
            id: format!("{provider}:default"),
            provider: provider_id.clone(),
            platform: Platform::Macos,
            version: None,
            data_roots: vec![workspace_path.into()],
            status: agenttidy_core::InstallationStatus::Available,
        };
        let sessions = session_cwds
            .iter()
            .enumerate()
            .map(|(index, cwd)| Session {
                id: SessionId::new(format!("session-{index}")),
                provider: provider_id.clone(),
                installation_id: installation.id.clone(),
                title: None,
                project: Some(ProjectRef::from_cwd(*cwd)),
                created_at: None,
                updated_at: None,
                lifecycle: SessionLifecycle::Inactive,
                size: SizeInfo::exact(0),
                resource_refs: vec![],
                metadata: serde_json::Map::new(),
            })
            .collect();
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "filesystem_eligible".into(),
            serde_json::Value::Bool(filesystem_eligible),
        );
        AgentSnapshot {
            installation: installation.clone(),
            sessions,
            resources: vec![Resource {
                id: ResourceId::new(format!("{provider}:workspace")),
                provider: provider_id,
                installation_id: installation.id,
                kind: ResourceKind::Workspace,
                locator: ResourceLocator::Dir {
                    path: workspace_path.into(),
                },
                ownership: Ownership::Exclusive,
                managed_by: ManagedBy::User,
                size: SizeInfo::exact(42),
                session_id: None,
                project: None,
                created_at: None,
                updated_at: None,
                dependencies: vec![],
                metadata,
            }],
            problems: BTreeMap::new(),
            completed_at: 0,
        }
    }

    #[test]
    fn exact_safe_codex_workspace_is_preview_eligible() {
        let snapshot = snapshot_with_workspace(
            ProviderId::CODEX,
            "/Users/example/Documents/Codex/task-a",
            &["/Users/example/Documents/Codex/task-a"],
            true,
        );
        let candidates = Application::workspace_cleanup_candidates_from_snapshots(&[snapshot]);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].eligible);
        assert_eq!(candidates[0].session_id.as_deref(), Some("session-0"));
    }

    #[test]
    fn workbuddy_workspace_stays_blocked_until_automation_references_are_known() {
        let snapshot = snapshot_with_workspace(
            ProviderId::WORKBUDDY,
            "/Users/example/WorkBuddy/task-a",
            &["/Users/example/WorkBuddy/task-a"],
            true,
        );
        let candidates = Application::workspace_cleanup_candidates_from_snapshots(&[snapshot]);
        assert!(!candidates[0].eligible);
        assert!(candidates[0]
            .blockers
            .iter()
            .any(|blocker| blocker.contains("automation references")));
    }

    /// Phase 7: with the frozen `automations.cwds` contract read into
    /// `automation_cwd_refs`, a workspace the automations do *not*
    /// reference loses the automation blocker (the remaining
    /// eligibility rules still apply); one they *do* reference stays
    /// blocked with the targeted reason.
    #[test]
    fn workbuddy_workspace_automation_refs_decide_the_blocker() {
        let mut snapshot = snapshot_with_workspace(
            ProviderId::WORKBUDDY,
            "/Users/example/WorkBuddy/task-a",
            &["/Users/example/WorkBuddy/task-a"],
            true,
        );
        let workspace = &mut snapshot.resources[0];
        workspace.metadata.insert(
            "automation_cwd_refs".into(),
            serde_json::Value::Array(vec![serde_json::Value::String(
                "/Users/example/other/project".into(),
            )]),
        );
        let candidates = Application::workspace_cleanup_candidates_from_snapshots(&[snapshot]);
        assert!(
            candidates[0]
                .blockers
                .iter()
                .all(|blocker| !blocker.contains("automation")),
            "unreferenced cwd must not carry an automation blocker: {:?}",
            candidates[0].blockers
        );
        assert!(candidates[0].eligible);

        // Referenced cwd ⇒ blocked with the targeted reason.
        let mut snapshot = snapshot_with_workspace(
            ProviderId::WORKBUDDY,
            "/Users/example/WorkBuddy/task-a",
            &["/Users/example/WorkBuddy/task-a"],
            true,
        );
        let workspace = &mut snapshot.resources[0];
        workspace.metadata.insert(
            "automation_cwd_refs".into(),
            serde_json::Value::Array(vec![serde_json::Value::String(
                "/Users/example/WorkBuddy/task-a".into(),
            )]),
        );
        let candidates = Application::workspace_cleanup_candidates_from_snapshots(&[snapshot]);
        assert!(!candidates[0].eligible);
        assert!(candidates[0]
            .blockers
            .iter()
            .any(|blocker| blocker.contains("automation references this cwd")));
    }

    #[test]
    fn multiple_session_matches_are_blocked() {
        let snapshot = snapshot_with_workspace(
            ProviderId::CODEX,
            "/Users/example/Documents/Codex/task-a",
            &[
                "/Users/example/Documents/Codex/task-a",
                "/Users/example/Documents/Codex/task-a",
            ],
            true,
        );
        let candidates = Application::workspace_cleanup_candidates_from_snapshots(&[snapshot]);
        assert!(!candidates[0].eligible);
        assert!(candidates[0]
            .blockers
            .iter()
            .any(|blocker| blocker.contains("exactly one session")));
    }

    // -----------------------------------------------------------------
    // Phase 6 cleanup policy / plan / revalidate / execute tests.
    // -----------------------------------------------------------------

    use agenttidy_core::{
        CleanupLocator, CleanupOperation, CleanupPrecondition, FileIdentity, ResourceFingerprint,
    };

    /// Snapshot with one session (given lifecycle + last-activity) owning
    /// one Exclusive `FileSet` session resource — the shape a real
    /// transcript cleanup unit is built from (§9.5). Policy tests mutate
    /// the resource / session to violate single §11 conditions.
    fn cleanup_snapshot(lifecycle: SessionLifecycle, updated_at: Option<u64>) -> AgentSnapshot {
        let mut snapshot = snapshot_with_workspace(
            ProviderId::CODEX,
            "/Users/example/Documents/Codex/task-a",
            &[],
            true,
        );
        let provider_id = ProviderId::new(ProviderId::CODEX);
        let session_id = SessionId::new("session-cleanup-0");
        snapshot.sessions = vec![Session {
            id: session_id.clone(),
            provider: provider_id.clone(),
            installation_id: snapshot.installation.id.clone(),
            title: None,
            project: None,
            created_at: Some(1_600_000_000_000),
            updated_at,
            lifecycle,
            size: SizeInfo::exact(1024),
            resource_refs: vec![ResourceRef {
                id: ResourceId::new("codex:session-resource"),
                kind: ResourceKind::Session,
            }],
            metadata: serde_json::Map::new(),
        }];
        snapshot.resources = vec![Resource {
            id: ResourceId::new("codex:session-resource"),
            provider: provider_id,
            installation_id: snapshot.installation.id.clone(),
            kind: ResourceKind::Session,
            locator: ResourceLocator::FileSet {
                paths: vec!["/tmp/transcript.jsonl".into()],
            },
            ownership: Ownership::Exclusive,
            managed_by: ManagedBy::Agent,
            size: SizeInfo::exact(1024),
            session_id: Some(session_id),
            project: None,
            created_at: None,
            updated_at: None,
            dependencies: vec![],
            metadata: serde_json::Map::new(),
        }];
        snapshot
    }

    /// A FileSet unit linked to `snapshot`'s resource. The fingerprint is
    /// a placeholder — the policy never reads it; only `revalidate_item`
    /// does (tested separately below).
    fn unit_for(snapshot: &AgentSnapshot, kind: CleanupUnitKind) -> CleanupUnit {
        CleanupUnit {
            id: "unit:policy".into(),
            provider: ProviderId::new(ProviderId::CODEX),
            installation_id: snapshot.installation.id.clone(),
            kind,
            resources: vec![ResourceRef {
                id: ResourceId::new("codex:session-resource"),
                kind: ResourceKind::Session,
            }],
            estimated_reclaimable_bytes: 1024,
            fingerprint: ResourceFingerprint {
                path: "/tmp/transcript.jsonl".into(),
                identity: FileIdentity {
                    volume: 1,
                    index: 2,
                },
                size_bytes: 1024,
                mtime_ms: 1_700_000_000_000,
            },
            locator: CleanupLocator::FileSet {
                paths: vec!["/tmp/transcript.jsonl".into()],
            },
            preconditions: vec![CleanupPrecondition {
                kind: CleanupPreconditionKind::PathExists,
                description: "path still exists".into(),
            }],
            operation: CleanupOperation {
                action: CleanupAction::Trash,
            },
            resource_kind: ResourceKind::Session,
        }
    }

    fn context<'a>(
        snapshot: &'a AgentSnapshot,
        inspection: &'a ProviderInspection,
        now_ms: u64,
    ) -> CleanupContext<'a> {
        CleanupContext {
            snapshot,
            inspection,
            now_ms,
        }
    }

    #[test]
    fn policy_marks_inactive_session_unit_review_required() {
        // §11.2 / §7.2: an Inactive, exclusively-owned, trash-recoverable
        // unit is ReviewRequired — never auto-promoted to LowRisk (red
        // line #5: age / absence of archive is not permission).
        let snapshot = cleanup_snapshot(SessionLifecycle::Inactive, Some(1_700_000_000_000));
        let inspection = ProviderInspection::empty_for_testing();
        let ctx = context(&snapshot, &inspection, 1_700_000_000_000);
        let decision = Application::cleanup_policy_evaluate(
            &unit_for(&snapshot, CleanupUnitKind::FileSet),
            &ctx,
        );
        assert_eq!(decision.risk, RiskLevel::ReviewRequired);
        assert!(!decision.recommended);
        assert!(decision.blockers.is_empty());
    }

    #[test]
    fn policy_blocks_when_agent_is_running() {
        // §16.2: an agent process anywhere near the installation means
        // concurrent execution is unsafe — fail closed (red line #6).
        let snapshot = cleanup_snapshot(SessionLifecycle::Inactive, Some(1_700_000_000_000));
        let mut inspection = ProviderInspection::empty_for_testing();
        inspection.agent_running = true;
        let ctx = context(&snapshot, &inspection, 1_700_000_000_000);
        let decision = Application::cleanup_policy_evaluate(
            &unit_for(&snapshot, CleanupUnitKind::FileSet),
            &ctx,
        );
        assert_eq!(decision.risk, RiskLevel::Blocked);
        assert!(!decision.recommended);
        assert!(decision
            .blockers
            .iter()
            .any(|b| b.contains("agent is currently running")));
    }

    #[test]
    fn policy_blocks_on_scan_error_problem() {
        // An Error-severity scan problem means part of the scan could
        // not be trusted — no unit from that installation may pass (§16.2).
        let mut snapshot = cleanup_snapshot(SessionLifecycle::Inactive, Some(1_700_000_000_000));
        snapshot.problems.insert(
            "unreadable:/x".into(),
            agenttidy_core::ScanProblem::Error {
                message: "permission denied".into(),
                path: Some("/x".into()),
            },
        );
        let inspection = ProviderInspection::empty_for_testing();
        let ctx = context(&snapshot, &inspection, 1_700_000_000_000);
        let decision = Application::cleanup_policy_evaluate(
            &unit_for(&snapshot, CleanupUnitKind::FileSet),
            &ctx,
        );
        assert_eq!(decision.risk, RiskLevel::Blocked);
        assert!(decision
            .blockers
            .iter()
            .any(|b| b.contains("Error problem")));
    }

    #[test]
    fn policy_blocks_active_session_units() {
        // §11.1: an Active session's resources are never cleanable.
        let snapshot = cleanup_snapshot(SessionLifecycle::Active, Some(1_700_000_000_000));
        let inspection = ProviderInspection::empty_for_testing();
        let ctx = context(&snapshot, &inspection, 1_700_000_000_000);
        let decision = Application::cleanup_policy_evaluate(
            &unit_for(&snapshot, CleanupUnitKind::FileSet),
            &ctx,
        );
        assert_eq!(decision.risk, RiskLevel::Blocked);
        assert!(decision
            .blockers
            .iter()
            .any(|b| b.contains("Active (§11.1)")));
    }

    #[test]
    fn policy_blocks_unknown_lifecycle_units() {
        // §11.2 fail-closed: an Unknown lifecycle is indistinguishable
        // from Active — Blocked.
        let snapshot = cleanup_snapshot(SessionLifecycle::Unknown, None);
        let inspection = ProviderInspection::empty_for_testing();
        let ctx = context(&snapshot, &inspection, 1_700_000_000_000);
        let decision = Application::cleanup_policy_evaluate(
            &unit_for(&snapshot, CleanupUnitKind::FileSet),
            &ctx,
        );
        assert_eq!(decision.risk, RiskLevel::Blocked);
        assert!(decision.blockers.iter().any(|b| b.contains("Unknown")));
    }

    #[test]
    fn policy_blocks_units_without_snapshot_linkage() {
        // Red line #6 ("never act on what we do not understand"): a unit
        // that does not resolve to a scanned resource is Blocked.
        let snapshot = cleanup_snapshot(SessionLifecycle::Inactive, Some(1_700_000_000_000));
        let mut unit = unit_for(&snapshot, CleanupUnitKind::FileSet);
        unit.resources.clear();
        let inspection = ProviderInspection::empty_for_testing();
        let ctx = context(&snapshot, &inspection, 1_700_000_000_000);
        let decision = Application::cleanup_policy_evaluate(&unit, &ctx);
        assert_eq!(decision.risk, RiskLevel::Blocked);
        assert!(decision
            .blockers
            .iter()
            .any(|b| b.contains("does not resolve to a resource")));
    }

    #[test]
    fn policy_blocks_non_exclusive_ownership() {
        // §9.5: only Exclusive resources may form an executable unit —
        // shared ownership blocks even an otherwise-stale archive.
        let archived_at = Some(1_600_000_000_000);
        let mut snapshot =
            cleanup_snapshot(SessionLifecycle::Archived { archived_at }, archived_at);
        snapshot.resources[0].ownership = Ownership::Shared;
        let inspection = ProviderInspection::empty_for_testing();
        let ctx = context(&snapshot, &inspection, 1_700_000_000_000);
        let decision = Application::cleanup_policy_evaluate(
            &unit_for(&snapshot, CleanupUnitKind::FileSet),
            &ctx,
        );
        assert_eq!(decision.risk, RiskLevel::Blocked);
        assert!(decision
            .blockers
            .iter()
            .any(|b| b.contains("not exclusively owned")));
    }

    #[test]
    fn policy_blocks_units_with_dependencies() {
        // §7.3: shared / unknown dependencies (e.g. an index DB row) are
        // Blocked until a coordinated ProviderOperation exists.
        let archived_at = Some(1_600_000_000_000);
        let mut snapshot =
            cleanup_snapshot(SessionLifecycle::Archived { archived_at }, archived_at);
        snapshot.resources[0].dependencies = vec![ResourceRef {
            id: ResourceId::new("codex:db-row"),
            kind: ResourceKind::Database,
        }];
        let inspection = ProviderInspection::empty_for_testing();
        let ctx = context(&snapshot, &inspection, 1_700_000_000_000);
        let decision = Application::cleanup_policy_evaluate(
            &unit_for(&snapshot, CleanupUnitKind::FileSet),
            &ctx,
        );
        assert_eq!(decision.risk, RiskLevel::Blocked);
        assert!(decision.blockers.iter().any(|b| b.contains("dependency")));
    }

    #[test]
    fn policy_promotes_archived_stale_to_low_risk() {
        // §11.1: "Archived + inactive > 30d + exclusive + recoverable →
        // Low Risk" — the single LowRisk path in v0.1.
        let archived_at = Some(1_600_000_000_000);
        let snapshot = cleanup_snapshot(SessionLifecycle::Archived { archived_at }, archived_at);
        let inspection = ProviderInspection::empty_for_testing();
        // 30d+ after the last activity.
        let ctx = context(
            &snapshot,
            &inspection,
            1_600_000_000_000 + THIRTY_DAYS_MS + 1_000,
        );
        let decision = Application::cleanup_policy_evaluate(
            &unit_for(&snapshot, CleanupUnitKind::FileSet),
            &ctx,
        );
        assert_eq!(decision.risk, RiskLevel::LowRisk);
        assert!(decision.recommended);
        assert!(decision.blockers.is_empty());
    }

    #[test]
    fn policy_keeps_recent_archived_at_review_required() {
        // §11.1: "Archived + recent → Review Required" — archive alone
        // is a weighted signal, not permission (§3.3).
        let now = 1_700_000_000_000u64;
        let snapshot = cleanup_snapshot(
            SessionLifecycle::Archived {
                archived_at: Some(now),
            },
            Some(now),
        );
        let inspection = ProviderInspection::empty_for_testing();
        let ctx = context(&snapshot, &inspection, now + 5 * 24 * 60 * 60 * 1000);
        let decision = Application::cleanup_policy_evaluate(
            &unit_for(&snapshot, CleanupUnitKind::FileSet),
            &ctx,
        );
        assert_eq!(decision.risk, RiskLevel::ReviewRequired);
        assert!(!decision.recommended);
        assert!(decision.blockers.is_empty());
    }

    #[tokio::test]
    async fn plan_from_empty_provider_snapshots_is_empty_and_fingerprinted() {
        // An empty installation set still yields a complete preview contract
        // (id, TTL, fingerprint), regardless of which providers are enabled.
        let app = Application::new();
        let plan = app
            .cleanup_plan_from_snapshots(&[])
            .await
            .expect("plan builds");
        assert!(plan.items.is_empty());
        assert_eq!(plan.total_reclaimable_bytes, 0);
        assert_eq!(plan.risk_summary.low_risk, 0);
        assert!(plan.expires_at.is_some());
        assert_eq!(plan.fingerprint.len(), 64, "sha256 hex digest");
        // Plan round-trips through JSON — this is the IPC contract the
        // Tauri command returns and the GUI sends back at execute time.
        let json = serde_json::to_string(&plan).unwrap();
        let back: CleanupPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, plan);
    }

    #[tokio::test]
    async fn execute_rejects_fingerprint_mismatch_without_touching_anything() {
        // §12.1 + the two-confirmation guard: a stale or tampered plan
        // must fail closed before the first file is touched.
        let app = Application::new();
        let plan = app.cleanup_plan_from_snapshots(&[]).await.unwrap();
        let error = app
            .cleanup_execute(&plan, "not-the-real-fingerprint", &[])
            .await
            .expect_err("mismatched fingerprint must be rejected");
        assert!(error.to_string().contains("fingerprint mismatch"));
    }

    /// §12.2 TTL: an expired plan is uniformly stale at revalidation and
    /// refused outright at execute — never silently run. Enforced in the
    /// backend so raw IPC callers cannot bypass the GUI's expiry gate.
    #[tokio::test]
    async fn expired_plan_is_skipped_at_revalidate_and_rejected_at_execute() {
        let app = Application::new();
        let item = CleanupItem {
            evidence: Default::default(),
            unit_id: "unit-expired".into(),
            installation_id: "claude-code:cli".into(),
            provider: ProviderId::new(ProviderId::CLAUDE_CODE),
            fingerprint: ResourceFingerprint {
                path: "/tmp/never-touched.jsonl".into(),
                identity: FileIdentity {
                    volume: 1,
                    index: 2,
                },
                size_bytes: 1,
                mtime_ms: 0,
            },
            locator: CleanupLocator::File {
                path: "/tmp/never-touched.jsonl".into(),
            },
            estimated_reclaimable_bytes: 1,
            action: CleanupAction::Trash,
            risk: RiskLevel::ReviewRequired,
            reasons: vec![],
            preconditions: vec![],
        };
        let plan = CleanupPlan {
            id: "plan-expired".into(),
            scan_id: "scan-expired".into(),
            created_at: 0,
            expires_at: Some(1), // long past — expired
            items: vec![item.clone()],
            total_reclaimable_bytes: 1,
            risk_summary: RiskSummary::default(),
            fingerprint: "fingerprint-expired".into(),
        };

        let outcomes = app
            .cleanup_revalidate(&plan)
            .await
            .expect("revalidate succeeds");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].outcome, CleanupOutcome::Skipped);
        assert!(outcomes[0].reason.as_deref().unwrap().contains("expired"));

        let error = app
            .cleanup_execute(
                &plan,
                &plan.fingerprint,
                std::slice::from_ref(&item.unit_id),
            )
            .await
            .expect_err("expired plan must be refused at execute");
        assert!(error.to_string().contains("expired"));
    }

    #[tokio::test]
    async fn revalidate_reports_pending_for_absent_items_and_agent_running_guard() {
        let app = Application::new();
        let plan = app.cleanup_plan_from_snapshots(&[]).await.unwrap();
        let outcomes = app.cleanup_revalidate(&plan).await.unwrap();
        assert_eq!(outcomes.len(), plan.items.len());
    }

    /// Revalidation against a real filesystem: a file that changed size
    /// between plan and execute must be Skipped with SizeUnchanged
    /// (§12.2 trigger), never executed.
    #[tokio::test]
    async fn revalidate_skips_when_file_size_changed_since_plan() {
        let dir = std::env::temp_dir().join(format!(
            "agenttidy-revalidate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("target.txt");
        std::fs::write(&file, b"original").unwrap();
        let metadata = std::fs::symlink_metadata(&file).unwrap();
        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let item = CleanupItem {
            evidence: Default::default(),
            unit_id: "unit-size-change".into(),
            installation_id: "codex:default".into(),
            provider: ProviderId::new(ProviderId::CODEX),
            fingerprint: ResourceFingerprint {
                path: file.clone(),
                identity: FileIdentity {
                    volume: 1,
                    index: 2,
                },
                size_bytes: 999_999, // deliberately wrong: file is 8 bytes
                mtime_ms,
            },
            locator: CleanupLocator::File { path: file.clone() },
            estimated_reclaimable_bytes: 8,
            action: CleanupAction::Trash,
            risk: RiskLevel::ReviewRequired,
            reasons: vec![],
            preconditions: vec![],
        };

        let outcome = revalidate_item(&item, None);
        assert_eq!(outcome.outcome, CleanupOutcome::Skipped);
        assert_eq!(
            outcome.failed_precondition,
            Some(CleanupPreconditionKind::SizeUnchanged)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A plan item whose path vanished must be Skipped with PathExists
    /// (§12.2 "path changed" trigger).
    #[tokio::test]
    async fn revalidate_skips_when_path_vanished() {
        let missing = std::env::temp_dir().join(format!(
            "agenttidy-gone-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let item = CleanupItem {
            evidence: Default::default(),
            unit_id: "unit-gone".into(),
            installation_id: "codex:default".into(),
            provider: ProviderId::new(ProviderId::CODEX),
            fingerprint: ResourceFingerprint {
                path: missing.clone(),
                identity: FileIdentity {
                    volume: 1,
                    index: 2,
                },
                size_bytes: 10,
                mtime_ms: 0,
            },
            locator: CleanupLocator::File { path: missing },
            estimated_reclaimable_bytes: 10,
            action: CleanupAction::Trash,
            risk: RiskLevel::ReviewRequired,
            reasons: vec![],
            preconditions: vec![],
        };
        let outcome = revalidate_item(&item, None);
        assert_eq!(outcome.outcome, CleanupOutcome::Skipped);
        assert_eq!(
            outcome.failed_precondition,
            Some(CleanupPreconditionKind::PathExists)
        );
    }

    /// §16.2 revalidation guard: agent_running from a fresh inspect()
    /// preempts every filesystem check.
    #[tokio::test]
    async fn revalidate_prefers_agent_running_over_filesystem_checks() {
        let file = std::env::temp_dir().join(format!(
            "agenttidy-agent-running-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&file, b"data").unwrap();
        let item = CleanupItem {
            evidence: Default::default(),
            unit_id: "unit-agent-running".into(),
            installation_id: "codex:default".into(),
            provider: ProviderId::new(ProviderId::CODEX),
            fingerprint: ResourceFingerprint {
                path: file.clone(),
                identity: FileIdentity {
                    volume: 1,
                    index: 2,
                },
                size_bytes: 999, // also wrong, but agent_running wins
                mtime_ms: 0,
            },
            locator: CleanupLocator::File { path: file.clone() },
            estimated_reclaimable_bytes: 4,
            action: CleanupAction::Trash,
            risk: RiskLevel::ReviewRequired,
            reasons: vec![],
            preconditions: vec![],
        };
        let mut inspection = ProviderInspection::empty_for_testing();
        inspection.agent_running = true;
        let outcome = revalidate_item(&item, Some(&inspection));
        assert_eq!(outcome.outcome, CleanupOutcome::Skipped);
        assert_eq!(
            outcome.failed_precondition,
            Some(CleanupPreconditionKind::NoAgentRunning)
        );
        std::fs::remove_file(&file).ok();
    }

    /// Unchanged file ⇒ Pending — the only outcome `cleanup_execute`
    /// will trash.
    #[tokio::test]
    async fn revalidate_passes_when_fingerprint_matches() {
        let file = std::env::temp_dir().join(format!(
            "agenttidy-pending-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&file, b"12345678").unwrap();
        let metadata = std::fs::symlink_metadata(&file).unwrap();
        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        // The happy path must use the *real* volume/inode — a fabricated
        // identity would trip IdentityUnchanged before size/mtime checks.
        let real_identity =
            agenttidy_infrastructure::fs_probe::file_identity(&file).expect("file identity");
        let item = CleanupItem {
            evidence: Default::default(),
            unit_id: "unit-ok".into(),
            installation_id: "codex:default".into(),
            provider: ProviderId::new(ProviderId::CODEX),
            fingerprint: ResourceFingerprint {
                path: file.clone(),
                identity: FileIdentity {
                    volume: real_identity.volume,
                    index: real_identity.index,
                },
                size_bytes: 8,
                mtime_ms,
            },
            locator: CleanupLocator::File { path: file.clone() },
            estimated_reclaimable_bytes: 8,
            action: CleanupAction::Trash,
            risk: RiskLevel::ReviewRequired,
            reasons: vec![],
            preconditions: vec![],
        };
        let inspection = ProviderInspection::empty_for_testing();
        let outcome = revalidate_item(&item, Some(&inspection));
        assert_eq!(outcome.outcome, CleanupOutcome::Pending);
        assert_eq!(outcome.failed_precondition, None);
        std::fs::remove_file(&file).ok();
    }

    /// §9.5 atomicity at revalidation: a FileSet whose side directory is
    /// gone out-of-band is no longer the planned unit — Skipped with
    /// PathExists before any fingerprint check runs.
    #[test]
    fn revalidate_skips_when_secondary_member_missing() {
        let dir = std::env::temp_dir().join(format!(
            "agenttidy-fileset-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("t.jsonl");
        std::fs::write(&transcript, b"x").unwrap();
        let metadata = std::fs::symlink_metadata(&transcript).unwrap();
        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let real_identity =
            agenttidy_infrastructure::fs_probe::file_identity(&transcript).expect("identity");
        let gone_side_dir = dir.join("t"); // deliberately never created
        let item = CleanupItem {
            evidence: Default::default(),
            unit_id: "unit-fileset-member".into(),
            installation_id: "claude-code:cli".into(),
            provider: ProviderId::new(ProviderId::CLAUDE_CODE),
            fingerprint: ResourceFingerprint {
                path: transcript.clone(),
                identity: FileIdentity {
                    volume: real_identity.volume,
                    index: real_identity.index,
                },
                size_bytes: 1,
                mtime_ms,
            },
            locator: CleanupLocator::FileSet {
                paths: vec![transcript, gone_side_dir],
            },
            estimated_reclaimable_bytes: 1,
            action: CleanupAction::Trash,
            risk: RiskLevel::ReviewRequired,
            reasons: vec![],
            preconditions: vec![],
        };
        let outcome = revalidate_item(&item, None);
        assert_eq!(outcome.outcome, CleanupOutcome::Skipped);
        assert_eq!(
            outcome.failed_precondition,
            Some(CleanupPreconditionKind::PathExists)
        );
        assert!(outcome
            .reason
            .unwrap()
            .contains("unit member no longer exists"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------
    // §12.3 partial-failure aggregation rules (pure, no trash service).
    // -----------------------------------------------------------------

    #[test]
    fn aggregate_all_removed_does_not_claim_physical_disk_reclamation() {
        let results: Vec<PathTrashResult> = vec![
            (PathBuf::from("/a"), Ok(TrashOutcome::Removed)),
            (PathBuf::from("/b"), Ok(TrashOutcome::Removed)),
        ];
        let (outcome, reclaimed, reason) = aggregate_trash_results(&results);
        assert_eq!(outcome, CleanupOutcome::Removed);
        assert_eq!(reclaimed, 0);
        assert!(reason.unwrap().contains("does not free disk space"));
    }

    #[test]
    fn aggregate_all_gone_is_idempotent_skip() {
        // §12.3: "already moved must not be reported as a fresh success".
        let results: Vec<PathTrashResult> = vec![
            (PathBuf::from("/a"), Ok(TrashOutcome::AlreadyGone)),
            (
                PathBuf::from("/b"),
                Err(TrashError::NotFound(PathBuf::from("/b"))),
            ),
        ];
        let (outcome, reclaimed, reason) = aggregate_trash_results(&results);
        assert_eq!(outcome, CleanupOutcome::Skipped);
        assert_eq!(reclaimed, 0);
        assert!(reason.unwrap().contains("already gone"));
    }

    #[test]
    fn aggregate_partial_failure_is_reported_honestly() {
        // §12.3: partial movement is reported with both the moved count
        // and the verbatim failure — never as success or silent loss.
        let results: Vec<PathTrashResult> = vec![
            (PathBuf::from("/a"), Ok(TrashOutcome::Removed)),
            (
                PathBuf::from("/b"),
                Ok(TrashOutcome::PlatformError("locked".into())),
            ),
        ];
        let (outcome, reclaimed, reason) = aggregate_trash_results(&results);
        assert_eq!(outcome, CleanupOutcome::Failed);
        assert_eq!(reclaimed, 0);
        let reason = reason.unwrap();
        assert!(reason.contains("1/2 member(s) moved to trash before failure"));
        assert!(reason.contains("/b: locked"));
    }

    // -----------------------------------------------------------------
    // End-to-end execute (§12.1 guard → §12.3 outcomes → §16.3 audit).
    // -----------------------------------------------------------------

    // Tests for fresh IPC instances and execution-time safety use a deterministic
    // provider, so real running agents never mask a dispatch regression (§18).
    mod live;
}
