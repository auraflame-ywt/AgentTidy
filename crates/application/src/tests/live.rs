//! Execution and revalidation regressions (`Start.md` §12 / §18).
//! Synthetic in-memory provider facts exercise the real Application pipeline
//! without spawning an installed agent or depending on its running state.

use super::*;
use std::ffi::OsString;
use std::sync::{Arc, Mutex};

static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct Probe {
    state: Arc<Mutex<ProbeState>>,
}

struct ProbeState {
    snapshot: AgentSnapshot,
    inspection: ProviderInspection,
    action: CleanupAction,
    validation_fails: bool,
    calls: usize,
}

#[async_trait::async_trait]
impl AgentProviderAdapter for Probe {
    fn id(&self) -> ProviderId {
        ProviderId::new(ProviderId::CODEX)
    }
    fn display_name(&self) -> &str {
        "Test provider"
    }
    async fn detect(&self) -> anyhow::Result<Vec<AgentInstallation>> {
        Ok(vec![self
            .state
            .lock()
            .unwrap()
            .snapshot
            .installation
            .clone()])
    }
    async fn inspect(&self, _: &AgentInstallation) -> anyhow::Result<ProviderInspection> {
        Ok(self.state.lock().unwrap().inspection.clone())
    }
    async fn capabilities(&self, _: &ProviderInspection) -> anyhow::Result<AgentCapabilities> {
        Ok(AgentCapabilities::empty())
    }
    async fn scan(&self, _: &AgentInstallation, _: &ScanOptions) -> anyhow::Result<AgentSnapshot> {
        Ok(self.state.lock().unwrap().snapshot.clone())
    }
    async fn build_cleanup_units(
        &self,
        snapshot: &AgentSnapshot,
    ) -> anyhow::Result<Vec<CleanupUnit>> {
        // Use production file-set construction to catch metadata regressions.
        let mut units = ClaudeCodeAdapter.build_cleanup_units(snapshot).await?;
        let action = self.state.lock().unwrap().action;
        for unit in &mut units {
            unit.provider = self.id();
            unit.operation.action = action;
            if action == CleanupAction::ProviderOperation {
                unit.kind = CleanupUnitKind::ProviderOperation;
                unit.locator = CleanupLocator::DbRow {
                    database: unit.fingerprint.path.clone(),
                    record_id: "session-cleanup-0".into(),
                };
            }
        }
        Ok(units)
    }
    async fn validate_cleanup_unit(
        &self,
        unit: &CleanupUnit,
        _: &AgentInstallation,
    ) -> anyhow::Result<Vec<CleanupPrecondition>> {
        anyhow::ensure!(
            !self.state.lock().unwrap().validation_fails,
            "writer lock or unsafe path"
        );
        Ok(unit.preconditions.clone())
    }
    async fn execute_provider_operation(
        &self,
        item: &CleanupItem,
        installation: &AgentInstallation,
    ) -> anyhow::Result<CleanupItemOutcome> {
        let mut state = self.state.lock().unwrap();
        anyhow::ensure!(
            installation == &state.snapshot.installation,
            "wrong installation"
        );
        // Mutation is only allowed while the cross-process guard is held.
        anyhow::ensure!(
            operation_guard::OperationGuard::acquire().is_err(),
            "guard not held"
        );
        state.calls += 1;
        // Model a completed provider operation; its session no longer exists.
        state.snapshot.resources.clear();
        state.snapshot.sessions.clear();
        Ok(CleanupItemOutcome {
            unit_id: item.unit_id.clone(),
            outcome: CleanupOutcome::Removed,
            reclaimed_bytes: 0,
            reason: None,
        })
    }
}

struct Harness {
    root: PathBuf,
    probe: Probe,
}
impl Harness {
    fn new(action: CleanupAction) -> Self {
        let root = std::env::temp_dir().join(format!(
            "agenttidy-live-{}-{}-{}",
            std::process::id(),
            ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = root.join("projects/test");
        std::fs::create_dir_all(project.join("session-cleanup-0")).unwrap();
        let transcript = project.join("session-cleanup-0.jsonl");
        std::fs::write(&transcript, b"metadata-only-test").unwrap();
        let mut snapshot = cleanup_snapshot(SessionLifecycle::Inactive, Some(0));
        snapshot.installation.data_roots = vec![root.to_string_lossy().into_owned()];
        snapshot.resources[0].locator = ResourceLocator::FileSet {
            paths: vec![
                transcript.to_string_lossy().into_owned(),
                project
                    .join("session-cleanup-0")
                    .to_string_lossy()
                    .into_owned(),
            ],
        };
        let probe = Probe {
            state: Arc::new(Mutex::new(ProbeState {
                snapshot,
                inspection: ProviderInspection::empty_for_testing(),
                action,
                validation_fails: false,
                calls: 0,
            })),
        };
        Self { root, probe }
    }
    fn app(&self) -> Application {
        let mut providers: ProviderRegistry<Box<dyn AgentProviderAdapter>> =
            ProviderRegistry::empty();
        providers
            .register(self.probe.id(), Box::new(self.probe.clone()))
            .unwrap();
        Application { providers }
    }
    async fn plan(&self) -> CleanupPlan {
        let app = self.app();
        let snapshots = app.scan(&ScanOptions::default()).await.unwrap();
        let plan = app.cleanup_plan_from_snapshots(&snapshots).await.unwrap();
        assert_eq!(plan.items.len(), 1);
        plan
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

/// Restore process-global home settings even when an assertion panics.
struct IsolatedHome {
    home: Option<OsString>,
    profile: Option<OsString>,
}
impl IsolatedHome {
    fn new(path: &std::path::Path) -> Self {
        std::fs::create_dir_all(path).unwrap();
        let previous = Self {
            home: std::env::var_os("HOME"),
            profile: std::env::var_os("USERPROFILE"),
        };
        std::env::set_var("HOME", path);
        std::env::set_var("USERPROFILE", path);
        previous
    }
}
impl Drop for IsolatedHome {
    fn drop(&mut self) {
        for (key, value) in [("HOME", &self.home), ("USERPROFILE", &self.profile)] {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

#[tokio::test]
async fn provider_operation_executes_after_plan_roundtrip_in_a_fresh_application() {
    let _serial = EXECUTE_ENV_MUTEX.lock().await;
    let harness = Harness::new(CleanupAction::ProviderOperation);
    let _home = IsolatedHome::new(&harness.root.join("home"));
    let plan: CleanupPlan =
        serde_json::from_str(&serde_json::to_string(&harness.plan().await).unwrap()).unwrap();
    let ids = vec![plan.items[0].unit_id.clone()];
    let results = harness
        .app()
        .cleanup_execute(&plan, &plan.fingerprint, &ids)
        .await
        .unwrap();
    assert_eq!(results[0].outcome, CleanupOutcome::Removed);
    assert_eq!(harness.probe.state.lock().unwrap().calls, 1);
    let rerun = harness
        .app()
        .cleanup_execute(&plan, &plan.fingerprint, &ids)
        .await
        .unwrap();
    assert_eq!(rerun[0].outcome, CleanupOutcome::Skipped);
    assert_eq!(harness.probe.state.lock().unwrap().calls, 1);
    let events = audit_log::read_all(&audit_log::audit_log_path().unwrap()).unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].outcome, CleanupOutcome::Pending);
    assert_eq!(events[1].outcome, CleanupOutcome::Removed);
    assert_eq!(events[2].outcome, CleanupOutcome::Skipped);
}

#[tokio::test]
async fn trash_execution_records_platform_failure_without_assuming_files_moved() {
    let _serial = EXECUTE_ENV_MUTEX.lock().await;
    let harness = Harness::new(CleanupAction::Trash);
    let _home = IsolatedHome::new(&harness.root.join("home"));
    let plan = harness.plan().await;
    let ids = vec![plan.items[0].unit_id.clone()];
    let results = harness
        .app()
        .cleanup_execute(&plan, &plan.fingerprint, &ids)
        .await
        .unwrap();
    assert!(matches!(
        results[0].outcome,
        CleanupOutcome::Removed | CleanupOutcome::Failed | CleanupOutcome::Skipped
    ));
    let rerun = harness
        .app()
        .cleanup_execute(&plan, &plan.fingerprint, &ids)
        .await
        .unwrap();
    if results[0].outcome == CleanupOutcome::Removed {
        assert_eq!(rerun[0].outcome, CleanupOutcome::Skipped);
    } else {
        // With no native trash service, the unchanged unit may fail again.
        assert_ne!(rerun[0].outcome, CleanupOutcome::Pending);
    }
    let events = audit_log::read_all(&audit_log::audit_log_path().unwrap()).unwrap();
    let finals: Vec<_> = events
        .iter()
        .filter(|event| event.outcome != CleanupOutcome::Pending)
        .collect();
    assert_eq!(finals.len(), 2);
    assert_eq!(finals[0].outcome, results[0].outcome);
}

#[tokio::test]
async fn revalidation_rejects_new_activity_schema_drift_and_provider_validation_failure() {
    let harness = Harness::new(CleanupAction::Trash);
    let plan = harness.plan().await;
    let initial = harness.app().cleanup_revalidate(&plan).await.unwrap();
    assert_eq!(initial[0].outcome, CleanupOutcome::Pending, "{initial:?}");
    harness.probe.state.lock().unwrap().snapshot.sessions[0].lifecycle = SessionLifecycle::Active;
    assert_eq!(
        harness.app().cleanup_revalidate(&plan).await.unwrap()[0].outcome,
        CleanupOutcome::Skipped
    );
    harness.probe.state.lock().unwrap().snapshot.sessions[0].lifecycle = SessionLifecycle::Inactive;
    harness
        .probe
        .state
        .lock()
        .unwrap()
        .inspection
        .schema_versions
        .insert("schema".into(), "changed".into());
    assert_eq!(
        harness.app().cleanup_revalidate(&plan).await.unwrap()[0].failed_precondition,
        Some(CleanupPreconditionKind::SchemaUnchanged)
    );
    harness
        .probe
        .state
        .lock()
        .unwrap()
        .inspection
        .schema_versions
        .clear();
    harness.probe.state.lock().unwrap().validation_fails = true;
    assert_eq!(
        harness.app().cleanup_revalidate(&plan).await.unwrap()[0].outcome,
        CleanupOutcome::Skipped
    );
}

#[tokio::test]
async fn revalidation_rejects_side_directory_changes_and_unverified_membership() {
    let harness = Harness::new(CleanupAction::Trash);
    let plan = harness.plan().await;
    let child = harness
        .root
        .join("projects/test/session-cleanup-0/tool-result");
    std::fs::write(&child, b"new-result").unwrap();
    assert_eq!(
        harness.app().cleanup_revalidate(&plan).await.unwrap()[0].outcome,
        CleanupOutcome::Skipped
    );
    let mut changed = plan.clone();
    changed.items[0].locator = CleanupLocator::File { path: child };
    assert_eq!(
        harness.app().cleanup_revalidate(&changed).await.unwrap()[0].outcome,
        CleanupOutcome::Skipped
    );
}

#[tokio::test]
async fn execute_refuses_modified_payload_even_when_confirmation_matches_supplied_hash() {
    let harness = Harness::new(CleanupAction::Trash);
    let mut plan = harness.plan().await;
    plan.items[0].action = CleanupAction::ProviderOperation;
    let error = harness
        .app()
        .cleanup_execute(&plan, &plan.fingerprint, &[plan.items[0].unit_id.clone()])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("payload fingerprint mismatch"));
    assert_eq!(harness.probe.state.lock().unwrap().calls, 0);
}

/// The Mac sandbox and ordinary permission errors must never turn an
/// unavailable audit path into an unaudited successful cleanup (§16.3).
#[tokio::test]
async fn unavailable_audit_destination_prevents_mutation() {
    let _serial = EXECUTE_ENV_MUTEX.lock().await;
    let harness = Harness::new(CleanupAction::ProviderOperation);
    let _home = IsolatedHome::new(&harness.root.join("home"));
    let plan = harness.plan().await;
    let directory = harness.root.join("home/.agenttidy");
    std::fs::create_dir_all(directory.join("operations.jsonl")).unwrap();
    let error = harness
        .app()
        .cleanup_execute(&plan, &plan.fingerprint, &[plan.items[0].unit_id.clone()])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("audit"));
    assert_eq!(harness.probe.state.lock().unwrap().calls, 0);
}

/// One invalid resource cannot hide all other providers in the Mac Review
/// screen. It remains visible, blocked, with its safety failure first.
#[tokio::test]
async fn unsafe_unit_becomes_a_blocked_preview_row() {
    let harness = Harness::new(CleanupAction::Trash);
    harness.probe.state.lock().unwrap().validation_fails = true;
    let plan = harness.plan().await;
    assert_eq!(plan.items[0].risk, RiskLevel::Blocked);
    assert!(plan.items[0].reasons[0].contains("safety validation failed"));
    assert_eq!(plan.total_reclaimable_bytes, 0);
}

/// Opt-in read-only local probe: reports counts only, never executes a plan
/// or prints session paths, titles or message content. It runs against actual
/// installations to catch cross-provider admission failures hidden by mocks.
#[tokio::test]
#[ignore = "reads local provider metadata; never mutates or cleans real data"]
async fn local_macos_readonly_scan_and_preview() {
    assert_eq!(
        agenttidy_infrastructure::paths::host_platform(),
        agenttidy_core::Platform::Macos
    );
    let app = Application::new();
    let snapshots = app.scan(&ScanOptions::default()).await.unwrap();
    let plan = app.cleanup_plan_from_snapshots(&snapshots).await.unwrap();
    for snapshot in &snapshots {
        let items: Vec<_> = plan
            .items
            .iter()
            .filter(|item| item.installation_id == snapshot.installation.id)
            .collect();
        eprintln!(
            "{}: {} sessions; {} units; {} blocked; {} diagnostics",
            snapshot.installation.id,
            snapshot.sessions.len(),
            items.len(),
            items
                .iter()
                .filter(|item| item.risk == RiskLevel::Blocked)
                .count(),
            snapshot.problems.len()
        );
    }
    assert!(plan
        .items
        .iter()
        .filter(|item| item.risk == RiskLevel::Blocked)
        .all(|item| !item.reasons.is_empty()));
}

/// Evidence failure is local to its row; a healthy resource in the same
/// snapshot must remain reviewable instead of losing the whole preview.
#[tokio::test]
async fn one_unsafe_member_does_not_hide_a_healthy_unit() {
    let harness = Harness::new(CleanupAction::Trash);
    let primary = harness.root.join("projects/test/bad-session.jsonl");
    std::fs::write(&primary, b"test metadata").unwrap();
    {
        let mut state = harness.probe.state.lock().unwrap();
        let mut resource = state.snapshot.resources[0].clone();
        resource.id = ResourceId::new("bad-resource");
        resource.session_id = Some(SessionId::new("bad-session"));
        resource.locator = ResourceLocator::FileSet {
            paths: vec![
                primary.to_string_lossy().into_owned(),
                harness
                    .root
                    .join("projects/test/missing-sidecar")
                    .to_string_lossy()
                    .into_owned(),
            ],
        };
        let mut session = state.snapshot.sessions[0].clone();
        session.id = SessionId::new("bad-session");
        session.resource_refs = vec![ResourceRef {
            id: resource.id.clone(),
            kind: ResourceKind::Session,
        }];
        state.snapshot.resources.push(resource);
        state.snapshot.sessions.push(session);
    }
    let app = harness.app();
    let snapshots = app.scan(&ScanOptions::default()).await.unwrap();
    let plan = app.cleanup_plan_from_snapshots(&snapshots).await.unwrap();
    assert_eq!(plan.items.len(), 2);
    assert_eq!(
        plan.items
            .iter()
            .filter(|item| item.risk == RiskLevel::Blocked)
            .count(),
        1
    );
    assert_eq!(
        plan.items
            .iter()
            .filter(|item| item.risk == RiskLevel::ReviewRequired)
            .count(),
        1
    );
}

/// Shared DB evidence cannot be rebased safely after an individual native
/// delete. Reject a conflicting batch in full, before even opening the audit.
#[tokio::test]
async fn shared_database_batch_is_rejected_before_any_operation() {
    let _serial = EXECUTE_ENV_MUTEX.lock().await;
    let harness = Harness::new(CleanupAction::ProviderOperation);
    let _home = IsolatedHome::new(&harness.root.join("home"));
    let mut plan = harness.plan().await;
    let mut second = plan.items[0].clone();
    second.unit_id.push_str("-second");
    if let agenttidy_core::CleanupLocator::DbRow { record_id, .. } = &mut second.locator {
        record_id.push_str("-second");
    }
    plan.items.push(second);
    plan.fingerprint = plan_fingerprint(
        &plan.id,
        &plan.scan_id,
        &plan.items,
        plan.total_reclaimable_bytes,
        &plan.risk_summary,
        plan.created_at,
        plan.expires_at,
    );
    let ids = plan
        .items
        .iter()
        .map(|item| item.unit_id.clone())
        .collect::<Vec<_>>();
    let error = harness
        .app()
        .cleanup_execute(&plan, &plan.fingerprint, &ids)
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("multiple selected operations share a database"));
    assert_eq!(harness.probe.state.lock().unwrap().calls, 0);
    assert!(!harness.root.join("home/.agenttidy").exists());
    // Unselected DB rows do not prevent an independently valid single choice.
    let results = harness
        .app()
        .cleanup_execute(&plan, &plan.fingerprint, &ids[..1])
        .await
        .unwrap();
    assert_eq!(results[0].outcome, CleanupOutcome::Removed);
    assert_eq!(results[1].outcome, CleanupOutcome::Skipped);
    assert_eq!(harness.probe.state.lock().unwrap().calls, 1);
}

/// Canonical aliases and hard links refer to the same database even when
/// their strings differ. Distinct DBs and unselected rows remain independent.
#[tokio::test]
async fn database_batch_admission_uses_live_file_identity() {
    let harness = Harness::new(CleanupAction::ProviderOperation);
    let mut plan = harness.plan().await;
    let mut second = plan.items[0].clone();
    second.unit_id.push_str("-second");
    let database = second.fingerprint.path.clone();
    let alias = database
        .parent()
        .unwrap()
        .join("../test")
        .join(database.file_name().unwrap());
    if let agenttidy_core::CleanupLocator::DbRow { database, .. } = &mut second.locator {
        *database = alias;
    }
    plan.items.push(second);
    let ids = plan
        .items
        .iter()
        .map(|item| item.unit_id.clone())
        .collect::<Vec<_>>();
    assert!(validate_database_batch(&plan, &ids).is_err());
    let hard_link = harness.root.join("same-database");
    std::fs::hard_link(&database, &hard_link).unwrap();
    if let agenttidy_core::CleanupLocator::DbRow { database, .. } = &mut plan.items[1].locator {
        *database = hard_link;
    }
    assert!(validate_database_batch(&plan, &ids).is_err());
    let independent = harness.root.join("independent-database");
    std::fs::write(&independent, b"independent sample").unwrap();
    if let agenttidy_core::CleanupLocator::DbRow { database, .. } = &mut plan.items[1].locator {
        *database = independent;
    }
    validate_database_batch(&plan, &ids).unwrap();
    validate_database_batch(&plan, &ids[..1]).unwrap();
}
