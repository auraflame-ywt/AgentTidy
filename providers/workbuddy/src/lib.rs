//! Read-only WorkBuddy provider adapter (`Start.md` §10, Phase 3/7).
//!
//! The adapter recognizes the Phase 0-verified session file set and,
//! since Phase 7, derives session lifecycle from the frozen `sessions`
//! v1 contract (`fixtures/workbuddy/db/sessions-schema.json`): a
//! soft-deleted row (`deleted_at IS NOT NULL`) maps to `Archived`, a
//! recognized terminal status maps to `Inactive`, anything else stays
//! `Unknown` so cleanup fail-closes. The database is opened through
//! the infrastructure read-only guard; it is never written and never
//! used to guess beyond the frozen contract
//! (`docs/safety/workbuddy-session-fileset.md`).

use agenttidy_core::{
    AgentCapabilities, AgentInstallation, AgentSnapshot, CapabilityStatus, CapabilityTopic,
    CleanupAction, CleanupItemOutcome, CleanupLocator, CleanupOperation, CleanupOutcome,
    CleanupPrecondition, CleanupPreconditionKind, CleanupUnit, CleanupUnitKind, FileIdentity,
    InstallationStatus, ManagedBy, Ownership, Platform, ProjectRef, ProviderId, Resource,
    ResourceFingerprint, ResourceId, ResourceKind, ResourceLocator, ResourceRef, ScanOptions,
    ScanProblem, Session, SessionId, SessionLifecycle, SizeConfidence, SizeInfo,
};
use agenttidy_infrastructure::disk_usage::UsageAccumulator;
use agenttidy_infrastructure::fs_probe::{file_identity, walk_tree, EntryKind, WalkProblem};
use agenttidy_infrastructure::processes::{any_process_running, ProcessSignature};
use agenttidy_infrastructure::sqlite::ReadOnlyDb;
use agenttidy_infrastructure::workspace_safety::inspect_workspace_safety;
use agenttidy_provider_api::{AgentProviderAdapter, ProviderInspection};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const INSTALLATION_ID: &str = "workbuddy:default";

/// Column set of the frozen `sessions` v1 contract
/// (`fixtures/workbuddy/db/sessions-schema.json`). A live database
/// whose `sessions` table does not carry exactly these columns is a
/// schema drift: lifecycle inference must not run against it
/// (fail-closed, `SchemaUnchanged`).
const SESSIONS_V1_COLUMNS: [&str; 12] = [
    "id",
    "cwd",
    "user_id",
    "title",
    "status",
    "created_at",
    "updated_at",
    "deleted_at",
    "source_mode",
    "model",
    "project_id",
    "buddy_snapshot_id",
];

/// Statuses observed in real WorkBuddy 5.5.6 data that mark a session
/// as terminal. `archived` is deliberately *not* special-cased for
/// permission: red line #5 — archive is intent, not permission — it
/// maps to the same `Inactive` lifecycle as the other terminal
/// statuses; only the user's own soft-delete (`deleted_at`) elevates
/// to `Archived`.
const TERMINAL_STATUSES: [&str; 4] = ["completed", "archived", "error", "terminated"];

/// The subset of a `sessions` row that lifecycle inference consumes
/// (frozen v1 contract — anything beyond these columns is never read).
#[derive(Debug, Clone, Copy)]
struct SessionRow {
    deleted_at: Option<i64>,
    status_is_terminal: bool,
    updated_at: Option<i64>,
}

/// Map one `sessions` row onto a `SessionLifecycle` per the table in
/// `docs/safety/workbuddy-session-fileset.md`:
///
/// - `deleted_at IS NOT NULL` ⇒ `Archived { archived_at: updated_at }`
///   (the user already deleted the session in the app; the transcript
///   is the on-disk leftover);
/// - `deleted_at IS NULL` + recognized terminal status ⇒ `Inactive`;
/// - anything else (live/running/unrecognized status, or no row at
///   all) ⇒ `Unknown` — cleanup stays fail-closed (§11.2).
fn lifecycle_from_row(row: Option<SessionRow>) -> SessionLifecycle {
    match row {
        Some(row) if row.deleted_at.is_some() => SessionLifecycle::Archived {
            archived_at: row.updated_at.and_then(|value| u64::try_from(value).ok()),
        },
        Some(row) if row.status_is_terminal => SessionLifecycle::Inactive,
        _ => SessionLifecycle::Unknown,
    }
}

#[derive(Debug, Default)]
pub struct WorkBuddyAdapter;

impl WorkBuddyAdapter {
    fn home_dir() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            std::env::var_os("USERPROFILE").map(PathBuf::from)
        }
        #[cfg(not(windows))]
        {
            std::env::var_os("HOME").map(PathBuf::from)
        }
    }

    fn platform() -> Platform {
        #[cfg(windows)]
        {
            Platform::Windows
        }
        #[cfg(not(windows))]
        {
            Platform::Macos
        }
    }

    fn running() -> bool {
        any_process_running(&[ProcessSignature::new(
            &["WorkBuddy", "workbuddy"],
            &["workbuddy.app/contents/"],
        )])
    }

    fn readable(path: &Path) -> bool {
        fs::read_dir(path).is_ok()
    }

    fn first_line(path: &Path) -> Result<WorkBuddyLine, String> {
        let file = fs::File::open(path).map_err(|error| error.to_string())?;
        let line = BufReader::new(file)
            .lines()
            .next()
            .ok_or_else(|| "empty transcript".to_string())?
            .map_err(|error| error.to_string())?;
        // The typed shape omits message/providerData, so transcript bodies
        // are discarded while only session identity and cwd are retained.
        serde_json::from_str(&line).map_err(|error| error.to_string())
    }

    fn measure(paths: &[PathBuf]) -> (SizeInfo, BTreeMap<String, ScanProblem>) {
        let mut usage = UsageAccumulator::new();
        let mut problems = BTreeMap::new();
        for path in paths {
            let outcome = walk_tree(path);
            for problem in outcome.problems {
                let WalkProblem::Unreadable { path, error } = problem;
                problems.insert(
                    format!("unreadable:{}", path.display()),
                    ScanProblem::Error {
                        message: error,
                        path: Some(path.display().to_string()),
                    },
                );
            }
            for entry in outcome
                .entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::File)
            {
                usage.add_file(entry);
            }
        }
        let usage = usage.finish();
        (
            SizeInfo {
                logical_bytes: usage.logical_bytes,
                allocated_bytes: usage.allocated_bytes,
                exclusive_bytes: Some(usage.logical_bytes),
                confidence: if usage.unidentifiable_files == 0 {
                    SizeConfidence::Exact
                } else {
                    SizeConfidence::Estimated
                },
                ..SizeInfo::default()
            },
            problems,
        )
    }

    fn scan_projects(
        root: &Path,
        installation: &AgentInstallation,
        options: &ScanOptions,
    ) -> (Vec<Session>, Vec<Resource>, BTreeMap<String, ScanProblem>) {
        let mut sessions = Vec::new();
        let mut resources = Vec::new();
        let mut problems = BTreeMap::new();
        let projects = root.join("projects");
        if !projects.is_dir() {
            problems.insert(
                "projects-missing".into(),
                ScanProblem::Warning {
                    message: "projects directory missing".into(),
                },
            );
            return (sessions, resources, problems);
        }
        let outcome = walk_tree(&projects);
        for problem in outcome.problems {
            let WalkProblem::Unreadable { path, error } = problem;
            problems.insert(
                format!("unreadable:{}", path.display()),
                ScanProblem::Error {
                    message: error,
                    path: Some(path.display().to_string()),
                },
            );
        }
        for entry in outcome.entries {
            let direct = entry.kind == EntryKind::File
                && entry.path.extension().and_then(|part| part.to_str()) == Some("jsonl")
                && entry
                    .path
                    .strip_prefix(&projects)
                    .ok()
                    .is_some_and(|relative| relative.components().count() == 2);
            if !direct {
                continue;
            }
            let path_text = entry.path.display().to_string();
            let line = match Self::first_line(&entry.path) {
                Ok(line) if line.session_id.is_some() && line.cwd.is_some() => line,
                Ok(_) => {
                    Self::unknown(
                        &entry.path,
                        installation,
                        options,
                        &mut resources,
                        &mut problems,
                        "session line lacks sessionId or cwd",
                    );
                    continue;
                }
                Err(error) => {
                    Self::unknown(
                        &entry.path,
                        installation,
                        options,
                        &mut resources,
                        &mut problems,
                        &error,
                    );
                    continue;
                }
            };
            let id = line.session_id.expect("checked above");
            if entry.path.file_stem().and_then(|part| part.to_str()) != Some(id.as_str()) {
                Self::unknown(
                    &entry.path,
                    installation,
                    options,
                    &mut resources,
                    &mut problems,
                    "sessionId does not match transcript filename",
                );
                continue;
            }
            let project_dir = entry.path.parent().expect("transcript has project parent");
            let mut paths = vec![entry.path.clone()];
            for candidate in [
                project_dir.join(&id),
                project_dir.join(format!("{id}.meta.json")),
                project_dir.join(format!("{id}.file-rollback.ndjson")),
            ] {
                if candidate.exists() {
                    paths.push(candidate);
                }
            }
            let (size, measure_problems) = Self::measure(&paths);
            problems.extend(measure_problems);
            let session_id = SessionId::new(id.clone());
            let resource_id = ResourceId::new(format!("{INSTALLATION_ID}:session:{path_text}"));
            let project = Some(ProjectRef::from_cwd(line.cwd.expect("checked above")));
            resources.push(Resource {
                id: resource_id.clone(),
                provider: ProviderId::new(ProviderId::WORKBUDDY),
                installation_id: installation.id.clone(),
                kind: ResourceKind::Session,
                locator: ResourceLocator::FileSet {
                    paths: paths
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect(),
                },
                ownership: Ownership::Exclusive,
                managed_by: ManagedBy::Agent,
                size,
                session_id: Some(session_id.clone()),
                project: project.clone(),
                created_at: None,
                updated_at: None,
                dependencies: vec![],
                metadata: serde_json::Map::new(),
            });
            sessions.push(Session {
                id: session_id,
                provider: ProviderId::new(ProviderId::WORKBUDDY),
                installation_id: installation.id.clone(),
                title: None,
                project,
                created_at: None,
                updated_at: None,
                // Without the DB contract we cannot prove deleted/archived or
                // inactivity. Unknown keeps future cleanup fail-closed.
                lifecycle: SessionLifecycle::Unknown,
                size,
                resource_refs: vec![ResourceRef {
                    id: resource_id,
                    kind: ResourceKind::Session,
                }],
                metadata: serde_json::Map::new(),
            });
        }
        (sessions, resources, problems)
    }

    fn unknown(
        path: &Path,
        installation: &AgentInstallation,
        options: &ScanOptions,
        resources: &mut Vec<Resource>,
        problems: &mut BTreeMap<String, ScanProblem>,
        reason: impl Into<String>,
    ) {
        let path = path.display().to_string();
        problems.insert(
            format!("unrecognized-transcript:{path}"),
            ScanProblem::Warning {
                message: reason.into(),
            },
        );
        if options.include_unknown {
            resources.push(Resource {
                id: ResourceId::new(format!("{INSTALLATION_ID}:unknown:{path}")),
                provider: ProviderId::new(ProviderId::WORKBUDDY),
                installation_id: installation.id.clone(),
                kind: ResourceKind::Unknown,
                locator: ResourceLocator::File { path },
                ownership: Ownership::Unknown,
                managed_by: ManagedBy::Agent,
                size: SizeInfo::unknown(),
                session_id: None,
                project: None,
                created_at: None,
                updated_at: None,
                dependencies: vec![],
                metadata: serde_json::Map::new(),
            });
        }
    }

    /// Open `workbuddy.db` read-only and return the connection when it
    /// passes the integrity gate. `None` (with a problem recorded)
    /// keeps every lifecycle `Unknown` — the fail-closed direction.
    fn open_sessions_db(
        root: &Path,
        problems: &mut BTreeMap<String, ScanProblem>,
    ) -> Option<ReadOnlyDb> {
        let db_path = root.join("workbuddy.db");
        match ReadOnlyDb::open(&db_path) {
            Ok(db) => {
                if !db.quick_check_ok().unwrap_or(false) {
                    problems.insert(
                        "workbuddy-db-quick-check".into(),
                        ScanProblem::Warning {
                            message: "workbuddy.db failed quick_check; lifecycle inference \
                                      skipped (fail-closed)"
                                .into(),
                        },
                    );
                    return None;
                }
                Some(db)
            }
            Err(error) => {
                problems.insert(
                    "workbuddy-db-unavailable".into(),
                    ScanProblem::Warning {
                        message: error.to_string(),
                    },
                );
                None
            }
        }
    }

    /// Verify the live `sessions` table matches the frozen v1 column
    /// contract. Drizzle-managed schemas evolve; a drift must stop
    /// lifecycle inference rather than misread a renamed column
    /// (`SchemaUnchanged`, §12.2).
    fn sessions_contract_holds(db: &ReadOnlyDb) -> bool {
        let conn = db.connection();
        let Ok(mut statement) = conn.prepare("SELECT name FROM pragma_table_info('sessions')")
        else {
            return false;
        };
        let columns: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default();
        SESSIONS_V1_COLUMNS
            .iter()
            .all(|expected| columns.iter().any(|column| column == expected))
    }

    /// Read the lifecycle-relevant subset of every `sessions` row,
    /// keyed by session id. Only the frozen v1 columns are selected;
    /// `quick_check` + the contract check gate the read.
    fn sessions_rows(db: &ReadOnlyDb) -> BTreeMap<String, SessionRow> {
        let conn = db.connection();
        let Ok(mut statement) =
            conn.prepare("SELECT id, deleted_at, status, updated_at FROM sessions")
        else {
            return BTreeMap::new();
        };
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    SessionRow {
                        deleted_at: row.get(1)?,
                        status_is_terminal: row.get::<_, Option<String>>(2)?.is_some_and(
                            |status| {
                                TERMINAL_STATUSES
                                    .iter()
                                    .any(|terminal| status.eq_ignore_ascii_case(terminal))
                            },
                        ),
                        updated_at: row.get(3)?,
                    },
                ))
            })
            .map(|rows| rows.flatten().collect::<Vec<_>>());
        rows.unwrap_or_default().into_iter().collect()
    }

    /// Read every automation's `cwds` JSON array and flatten the
    /// referenced paths. Used only to decide whether the default
    /// workspace is referenced (⇒ blocked) or not — never to clean
    /// inside the automations tables themselves.
    fn automation_cwds(db: &ReadOnlyDb) -> Vec<String> {
        let conn = db.connection();
        let Ok(mut statement) = conn.prepare("SELECT cwds FROM automations") else {
            return Vec::new();
        };
        let arrays: Vec<Option<String>> = statement
            .query_map([], |row| row.get(0))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default();
        let mut cwds = Vec::new();
        for array in arrays.into_iter().flatten() {
            if let Ok(paths) = serde_json::from_str::<Vec<String>>(&array) {
                cwds.extend(paths);
            }
        }
        cwds
    }

    /// §12.2 invalidation triggers for a WorkBuddy session file set
    /// (`docs/safety/workbuddy-session-fileset.md`).
    fn fileset_preconditions() -> Vec<CleanupPrecondition> {
        vec![
            CleanupPrecondition {
                kind: CleanupPreconditionKind::PathExists,
                description: "transcript and listed sidecars still exist".into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::PathUnchanged,
                description: "paths canonicalize to the scanned locations (§16.1)".into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::IdentityUnchanged,
                description: "transcript file identity unchanged".into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::SizeUnchanged,
                description: "transcript size unchanged".into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::MtimeUnchanged,
                description: "transcript mtime unchanged — a session that reactivates \
                              writes its transcript"
                    .into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::NoAgentRunning,
                description: "no WorkBuddy process detected (§16.2)".into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::SessionInactive,
                description: "owning session lifecycle is still Inactive/Archived".into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::SchemaUnchanged,
                description: "workbuddy.db sessions contract still v1".into(),
            },
        ]
    }
}

#[derive(Debug, Deserialize)]
struct WorkBuddyLine {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
}

#[async_trait::async_trait]
impl AgentProviderAdapter for WorkBuddyAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::new(ProviderId::WORKBUDDY)
    }
    fn display_name(&self) -> &str {
        "WorkBuddy"
    }

    async fn detect(&self) -> anyhow::Result<Vec<AgentInstallation>> {
        let Some(home) = Self::home_dir() else {
            return Ok(vec![]);
        };
        let state_root = home.join(".workbuddy");
        if !state_root.exists() {
            return Ok(vec![]);
        }
        let workspace = home.join("WorkBuddy");
        let mut data_roots = vec![state_root.display().to_string()];
        if workspace.exists() {
            data_roots.push(workspace.display().to_string());
        }
        Ok(vec![AgentInstallation {
            id: INSTALLATION_ID.into(),
            provider: self.id(),
            platform: Self::platform(),
            version: None,
            data_roots,
            status: if Self::readable(&state_root) {
                InstallationStatus::Available
            } else {
                InstallationStatus::PermissionRequired
            },
        }])
    }

    async fn inspect(
        &self,
        installation: &AgentInstallation,
    ) -> anyhow::Result<ProviderInspection> {
        let mut journal_modes = BTreeMap::new();
        let mut schema_versions = BTreeMap::new();
        let mut problems = BTreeMap::new();
        if let Some(root) = installation.data_roots.first() {
            let db_path = Path::new(root).join("workbuddy.db");
            match ReadOnlyDb::open(&db_path) {
                Ok(db) => {
                    match db.journal_mode() {
                        Ok(mode) => {
                            journal_modes.insert("workbuddy".into(), mode);
                        }
                        Err(error) => {
                            problems.insert(
                                "workbuddy-db-journal".into(),
                                ScanProblem::Warning {
                                    message: error.to_string(),
                                },
                            );
                        }
                    }
                    // Phase 7: surface the frozen `sessions` v1 contract
                    // state so the revalidator can enforce
                    // `SchemaUnchanged` (§12.2). A drift is a warning +
                    // no schema entry — cleanup fail-closes on the
                    // missing entry, never on a guess.
                    if Self::sessions_contract_holds(&db) {
                        schema_versions.insert("workbuddy.sessions".into(), "v1".into());
                    } else {
                        problems.insert(
                            "workbuddy-sessions-schema-unknown".into(),
                            ScanProblem::Warning {
                                message: "sessions table does not match the frozen v1 \
                                          contract; lifecycle inference skipped (fail-closed)"
                                    .into(),
                            },
                        );
                    }
                }
                Err(error) => {
                    problems.insert(
                        "workbuddy-db-unavailable".into(),
                        ScanProblem::Warning {
                            message: error.to_string(),
                        },
                    );
                }
            }
        }
        Ok(ProviderInspection {
            version: installation.version.clone(),
            schema_versions,
            journal_modes,
            agent_running: Self::running(),
            readable_roots: installation
                .data_roots
                .iter()
                .map(|root| (root.clone(), Self::readable(Path::new(root))))
                .collect(),
            unknown_structures: vec![],
            problems,
        })
    }

    async fn capabilities(
        &self,
        inspection: &ProviderInspection,
    ) -> anyhow::Result<AgentCapabilities> {
        let readable = !inspection.readable_roots.is_empty()
            && inspection.readable_roots.values().all(|value| *value);
        let status = if readable {
            CapabilityStatus::ReadOnly
        } else {
            CapabilityStatus::PermissionRequired
        };
        Ok(AgentCapabilities::build([
            (CapabilityTopic::Sessions, status),
            (CapabilityTopic::Projects, status),
        ]))
    }

    async fn scan(
        &self,
        installation: &AgentInstallation,
        options: &ScanOptions,
    ) -> anyhow::Result<AgentSnapshot> {
        let root = installation
            .data_roots
            .first()
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("WorkBuddy installation has no state root"))?;
        let (mut sessions, mut resources, mut problems) =
            Self::scan_projects(&root, installation, options);

        // Phase 7: apply the frozen `sessions` v1 contract to derive
        // lifecycle. Missing/unreadable/drifting DB leaves everything
        // `Unknown` (fail-closed, §11.2) — the scan still lists the
        // sessions, it just cannot open them for cleanup.
        let mut automation_cwds: Option<Vec<String>> = None;
        if let Some(db) = Self::open_sessions_db(&root, &mut problems) {
            if Self::sessions_contract_holds(&db) {
                let rows = Self::sessions_rows(&db);
                for session in &mut sessions {
                    let row = rows.get(session.id.as_str()).copied();
                    session.lifecycle = lifecycle_from_row(row);
                    // Timestamps double as metadata for the sessions view.
                    if let Some(row) = row {
                        if let Some(updated_at) =
                            row.updated_at.and_then(|value| u64::try_from(value).ok())
                        {
                            session.updated_at = Some(updated_at);
                        }
                    }
                }
                // Automations reference cwds; expose the union so the
                // workspace policy can block a referenced cwd.
                automation_cwds = Some(Self::automation_cwds(&db));
            }
        }

        // Default workspaces can mix user artifacts with nested agent state.
        // Report their footprint, but keep ownership Unknown and cleanup blocked.
        if let Some(workspace) = installation.data_roots.get(1).map(PathBuf::from) {
            if workspace.is_dir() {
                let (size, _) = Self::measure(std::slice::from_ref(&workspace));
                let facts = inspect_workspace_safety(&workspace);
                let mut metadata = serde_json::Map::new();
                metadata.insert(
                    "filesystem_eligible".into(),
                    facts.is_filesystem_eligible().into(),
                );
                metadata.insert("has_git_marker".into(), facts.has_git_marker.into());
                metadata.insert("has_unsafe_entry".into(), facts.has_unsafe_entry.into());
                metadata.insert(
                    "has_unreadable_entry".into(),
                    facts.has_unreadable_entry.into(),
                );
                // Phase 7: when the automations contract was readable,
                // surface the referenced cwds so the workspace policy can
                // decide "referenced (blocked)" vs "not referenced
                // (eligible)". Absent metadata keeps the conservative
                // "not yet verified" blocker.
                if let Some(cwds) = automation_cwds {
                    metadata.insert(
                        "automation_cwd_refs".into(),
                        serde_json::Value::Array(
                            cwds.into_iter().map(serde_json::Value::String).collect(),
                        ),
                    );
                }
                resources.push(Resource {
                    id: ResourceId::new(format!(
                        "{INSTALLATION_ID}:workspace:{}",
                        workspace.display()
                    )),
                    provider: ProviderId::new(ProviderId::WORKBUDDY),
                    installation_id: installation.id.clone(),
                    kind: ResourceKind::Workspace,
                    locator: ResourceLocator::Dir {
                        path: workspace.display().to_string(),
                    },
                    ownership: Ownership::Unknown,
                    managed_by: ManagedBy::User,
                    size,
                    session_id: None,
                    project: None,
                    created_at: None,
                    updated_at: None,
                    dependencies: vec![],
                    metadata,
                });
            }
        }
        Ok(AgentSnapshot {
            installation: installation.clone(),
            sessions,
            resources,
            problems,
            completed_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64,
        })
    }

    /// Phase 7 beta: each exclusively-owned session resource with a
    /// DB-derived `Inactive`/`Archived` lifecycle emits one `FileSet`
    /// cleanup unit (transcript + side dir + `.meta.json` +
    /// `.file-rollback.ndjson` — exactly what the scan measured).
    /// Units fail-closed when the `sessions` v1 contract cannot be
    /// verified: no DB, no contract, no units
    /// (`docs/safety/workbuddy-session-fileset.md`).
    async fn build_cleanup_units(
        &self,
        snapshot: &AgentSnapshot,
    ) -> anyhow::Result<Vec<CleanupUnit>> {
        let mut units = Vec::new();
        let root = match snapshot.installation.data_roots.first() {
            Some(root) => Path::new(root),
            None => return Ok(units),
        };
        // Gate on the DB + frozen contract: without it the lifecycle
        // stayed `Unknown` and §11.2 would block every unit anyway —
        // emitting none keeps the plan honest about what is provable.
        let mut problems = BTreeMap::new();
        let Some(db) = Self::open_sessions_db(root, &mut problems) else {
            return Ok(units);
        };
        if !Self::sessions_contract_holds(&db) {
            return Ok(units);
        }
        let rows = Self::sessions_rows(&db);
        drop(db);

        for resource in &snapshot.resources {
            if resource.kind != ResourceKind::Session || resource.ownership != Ownership::Exclusive
            {
                continue;
            }
            let Some(session_id) = &resource.session_id else {
                continue;
            };
            // Cross-check the row the same way scan derived the
            // lifecycle — a row that vanished or flipped to live
            // since the scan drops the unit (fail-closed).
            let row = rows.get(session_id.as_str()).copied();
            if !matches!(
                lifecycle_from_row(row),
                SessionLifecycle::Inactive | SessionLifecycle::Archived { .. }
            ) {
                continue;
            }
            let ResourceLocator::FileSet { paths } = &resource.locator else {
                continue;
            };
            let Some(primary) = paths.first().map(PathBuf::from) else {
                continue;
            };
            // The transcript carries the §12.2 fingerprint; a path
            // that cannot be fingerprinted cannot be revalidated.
            let Ok(metadata) = fs::symlink_metadata(&primary) else {
                continue;
            };
            let Some(identity) = file_identity(&primary) else {
                continue;
            };
            let mtime_ms = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|since_epoch| since_epoch.as_millis() as i64)
                .unwrap_or(0);
            units.push(CleanupUnit {
                id: format!(
                    "{}:session:{}",
                    snapshot.installation.id,
                    session_id.as_str()
                ),
                provider: ProviderId::new(ProviderId::WORKBUDDY),
                installation_id: snapshot.installation.id.clone(),
                kind: CleanupUnitKind::FileSet,
                resources: vec![ResourceRef {
                    id: resource.id.clone(),
                    kind: resource.kind,
                }],
                estimated_reclaimable_bytes: resource.size.logical_bytes,
                fingerprint: ResourceFingerprint {
                    path: primary,
                    identity: FileIdentity {
                        volume: identity.volume,
                        index: identity.index,
                    },
                    size_bytes: metadata.len(),
                    mtime_ms,
                },
                locator: CleanupLocator::FileSet {
                    paths: paths.iter().map(PathBuf::from).collect(),
                },
                preconditions: Self::fileset_preconditions(),
                operation: CleanupOperation {
                    action: CleanupAction::Trash,
                },
                resource_kind: ResourceKind::Session,
            });
        }
        Ok(units)
    }

    /// §16.1 path safety (shared helper) + the WorkBuddy DB cross-check:
    /// the `sessions` row must still exist and still map to
    /// `Inactive`/`Archived` under the frozen v1 contract. Any drift
    /// drops the unit's plan (fail-closed).
    async fn validate_cleanup_unit(
        &self,
        unit: &CleanupUnit,
        installation: &AgentInstallation,
    ) -> anyhow::Result<Vec<CleanupPrecondition>> {
        agenttidy_infrastructure::paths::validate_transcript_fileset_paths(unit, installation)?;

        let root = installation
            .data_roots
            .first()
            .ok_or_else(|| anyhow::anyhow!("WorkBuddy installation has no state root"))?;
        let record_id = unit.id.rsplit(':').next().unwrap_or_default().to_string();
        let mut problems = BTreeMap::new();
        let db = Self::open_sessions_db(Path::new(root), &mut problems)
            .ok_or_else(|| anyhow::anyhow!("workbuddy.db unavailable at validate time"))?;
        if !Self::sessions_contract_holds(&db) {
            anyhow::bail!("sessions table drifted from the frozen v1 contract");
        }
        let rows = Self::sessions_rows(&db);
        let row = rows.get(&record_id).copied();
        if !matches!(
            lifecycle_from_row(row),
            SessionLifecycle::Inactive | SessionLifecycle::Archived { .. }
        ) {
            anyhow::bail!(
                "sessions row for {record_id} no longer maps to Inactive/Archived \
                 (missing or live) — rescan before cleaning"
            );
        }
        Ok(unit.preconditions.clone())
    }

    /// WorkBuddy sessions are file-owned — `Trash` only. The trait's
    /// default `execute_provider_operation` (`Failed("not supported")`)
    /// is the correct behaviour; this override exists only to document
    /// that the default is intentional, not an omission.
    async fn execute_provider_operation(
        &self,
        item: &agenttidy_core::cleanup::CleanupItem,
        _installation: &AgentInstallation,
    ) -> anyhow::Result<CleanupItemOutcome> {
        Ok(CleanupItemOutcome {
            unit_id: item.unit_id.clone(),
            outcome: CleanupOutcome::Failed,
            reclaimed_bytes: 0,
            reason: Some(
                "WorkBuddy sessions are file-owned; use the Trash action, not a \
                 provider operation"
                    .into(),
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SystemTime granularity can be coarser than a parallel test start;
    // the counter keeps test roots disjoint even when timestamps
    // collide (same pattern as the Codex adapter tests).
    static TEMP_ROOT_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "agenttidy-workbuddy-{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            TEMP_ROOT_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("projects/project")).unwrap();
        root
    }
    fn installation(root: &Path) -> AgentInstallation {
        AgentInstallation {
            id: INSTALLATION_ID.into(),
            provider: ProviderId::new(ProviderId::WORKBUDDY),
            platform: WorkBuddyAdapter::platform(),
            version: None,
            data_roots: vec![root.display().to_string()],
            status: InstallationStatus::Available,
        }
    }
    #[test]
    fn scan_collects_verified_session_file_set() {
        let root = root();
        let project = root.join("projects/project");
        let id = "d493ed8f-0147-4c01-89df-b9da4b956532";
        fs::write(project.join(format!("{id}.jsonl")), format!("{{\"type\":\"message\",\"sessionId\":\"{id}\",\"cwd\":\"/tmp/project\",\"providerData\":{{\"secret\":\"ignored\"}}}}\n")).unwrap();
        fs::create_dir_all(project.join(id).join("tool-results")).unwrap();
        fs::write(project.join(format!("{id}.meta.json")), "{}").unwrap();
        let (sessions, resources, problems) =
            WorkBuddyAdapter::scan_projects(&root, &installation(&root), &ScanOptions::default());
        assert!(problems.is_empty());
        assert_eq!(sessions.len(), 1);
        assert!(matches!(sessions[0].lifecycle, SessionLifecycle::Unknown));
        assert_eq!(resources.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    /// Lay down a `workbuddy.db` carrying the frozen `sessions` v1
    /// contract (`fixtures/workbuddy/db/sessions-schema.json`) plus one
    /// row per requested lifecycle fixture. Shared by the Phase 7
    /// scan / cleanup-unit tests.
    fn lay_workbuddy_db(root: &Path, rows: &[(&str, Option<i64>, &str, Option<i64>)]) {
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
        for (id, deleted_at, status, updated_at) in rows {
            conn.execute(
                "INSERT INTO sessions (id, status, updated_at, deleted_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, status, updated_at, deleted_at],
            )
            .unwrap();
        }
    }

    /// `sessions_contract_holds` accepts the v1 column set and rejects
    /// a drifted table (renamed column ⇒ schema drift ⇒ fail-closed).
    #[test]
    fn sessions_contract_check_accepts_v1_and_rejects_drift() {
        let root = root();
        lay_workbuddy_db(&root, &[]);
        let mut problems = BTreeMap::new();
        let db = WorkBuddyAdapter::open_sessions_db(&root, &mut problems).unwrap();
        assert!(WorkBuddyAdapter::sessions_contract_holds(&db));
        drop(db);

        // Drift: drop a column the contract requires.
        let conn = rusqlite::Connection::open(root.join("workbuddy.db")).unwrap();
        // SQLite cannot DROP COLUMN before 3.35 — rebuild instead.
        conn.execute_batch(
            "CREATE TABLE drifted AS SELECT id, cwd, status, updated_at, deleted_at \
             FROM sessions;
             DROP TABLE sessions;
             ALTER TABLE drifted RENAME TO sessions;",
        )
        .unwrap();
        drop(conn);
        let db = WorkBuddyAdapter::open_sessions_db(&root, &mut problems).unwrap();
        assert!(
            !WorkBuddyAdapter::sessions_contract_holds(&db),
            "drifted table must fail the contract check"
        );
        // Close the database handle *before* removing the directory:
        // on Windows an open SQLite connection keeps a file lock on
        // `workbuddy.db`, and `remove_dir_all` would fail with
        // ERROR_SHARING_VIOLATION (error 32).
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }

    /// Scan derives lifecycle from the frozen v1 contract: a
    /// soft-deleted row (`deleted_at NOT NULL`) ⇒ `Archived`, a
    /// terminal status ⇒ `Inactive`, a live/unrecognized status ⇒
    /// `Unknown` (fail-closed).
    #[tokio::test]
    async fn scan_uses_sessions_table_to_derive_lifecycle() {
        let root = root();
        let project = root.join("projects/project");
        let ids = [
            "aaaaaaaa-0000-0000-0000-000000000001",
            "aaaaaaaa-0000-0000-0000-000000000002",
            "aaaaaaaa-0000-0000-0000-000000000003",
        ];
        for id in ids {
            fs::write(
                project.join(format!("{id}.jsonl")),
                format!(
                    "{{\"type\":\"message\",\"sessionId\":\"{id}\",\"cwd\":\"/tmp/project\"}}\n"
                ),
            )
            .unwrap();
        }
        lay_workbuddy_db(
            &root,
            &[
                // Soft-deleted by the user ⇒ Archived.
                (
                    ids[0],
                    Some(1_700_000_000_000),
                    "completed",
                    Some(1_700_000_000_000),
                ),
                // Terminal status, not deleted ⇒ Inactive.
                (ids[1], None, "completed", Some(1_700_000_000_000)),
                // Unrecognized status ⇒ Unknown (fail-closed).
                (ids[2], None, "running", Some(1_700_000_000_000)),
            ],
        );
        let snapshot = WorkBuddyAdapter
            .scan(&installation(&root), &ScanOptions::default())
            .await
            .unwrap();
        let lifecycle_of = |needle: &str| {
            snapshot
                .sessions
                .iter()
                .find(|session| session.id.as_str() == needle)
                .map(|session| session.lifecycle.clone())
        };
        assert!(matches!(
            lifecycle_of(ids[0]),
            Some(SessionLifecycle::Archived {
                archived_at: Some(_)
            })
        ));
        assert!(matches!(
            lifecycle_of(ids[1]),
            Some(SessionLifecycle::Inactive)
        ));
        assert!(matches!(
            lifecycle_of(ids[2]),
            Some(SessionLifecycle::Unknown)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    /// Without `workbuddy.db` the lifecycle stays `Unknown` — the
    /// scan still lists sessions but §11.2 keeps cleanup blocked
    /// (fail-closed).
    #[tokio::test]
    async fn scan_preserves_unknown_lifecycle_when_db_unavailable() {
        let root = root();
        let project = root.join("projects/project");
        let id = "aaaaaaaa-0000-0000-0000-000000000004";
        fs::write(
            project.join(format!("{id}.jsonl")),
            format!("{{\"type\":\"message\",\"sessionId\":\"{id}\",\"cwd\":\"/tmp/project\"}}\n"),
        )
        .unwrap();
        let snapshot = WorkBuddyAdapter
            .scan(&installation(&root), &ScanOptions::default())
            .await
            .unwrap();
        assert!(snapshot
            .sessions
            .iter()
            .all(|session| matches!(session.lifecycle, SessionLifecycle::Unknown)));
        assert!(snapshot.problems.contains_key("workbuddy-db-unavailable"));
        fs::remove_dir_all(root).unwrap();
    }

    /// `build_cleanup_units` emits one FileSet unit per session whose
    /// DB-derived lifecycle is `Inactive`/`Archived` — and none when
    /// the DB is unavailable (fail-closed).
    #[tokio::test]
    async fn build_cleanup_units_forms_one_fileset_unit_per_cleanable_session() {
        use agenttidy_core::cleanup::{CleanupAction, CleanupLocator, CleanupUnitKind};
        let root = root();
        let project = root.join("projects/project");
        let cleanable = "aaaaaaaa-0000-0000-0000-000000000011";
        let live = "aaaaaaaa-0000-0000-0000-000000000012";
        for id in [cleanable, live] {
            fs::write(
                project.join(format!("{id}.jsonl")),
                format!(
                    "{{\"type\":\"message\",\"sessionId\":\"{id}\",\"cwd\":\"/tmp/project\"}}\n"
                ),
            )
            .unwrap();
        }
        lay_workbuddy_db(
            &root,
            &[
                (cleanable, None, "completed", Some(1_700_000_000_000)),
                (live, None, "running", Some(1_700_000_000_000)),
            ],
        );
        let snapshot = WorkBuddyAdapter
            .scan(&installation(&root), &ScanOptions::default())
            .await
            .unwrap();
        let units = WorkBuddyAdapter
            .build_cleanup_units(&snapshot)
            .await
            .unwrap();
        assert_eq!(units.len(), 1, "only the Inactive session forms a unit");
        let unit = &units[0];
        assert_eq!(unit.kind, CleanupUnitKind::FileSet);
        assert_eq!(unit.operation.action, CleanupAction::Trash);
        let CleanupLocator::FileSet { paths } = &unit.locator else {
            panic!("expected FileSet locator");
        };
        assert!(
            paths
                .iter()
                .any(|p| p.ends_with(format!("{cleanable}.jsonl"))),
            "transcript must be a member: {paths:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// `validate_cleanup_unit` rejects (drops the plan) when the
    /// `sessions` row disappeared between scan and plan-build — the
    /// DB cross-check from `docs/safety/workbuddy-session-fileset.md`.
    #[tokio::test]
    async fn validate_cleanup_unit_rejects_when_sessions_row_missing() {
        let root = root();
        let project = root.join("projects/project");
        let id = "aaaaaaaa-0000-0000-0000-000000000013";
        let transcript = project.join(format!("{id}.jsonl"));
        fs::write(
            &transcript,
            format!("{{\"type\":\"message\",\"sessionId\":\"{id}\",\"cwd\":\"/tmp/project\"}}\n"),
        )
        .unwrap();
        // DB exists with the v1 contract but NO row for this session.
        lay_workbuddy_db(&root, &[]);
        let installation = installation(&root);
        let unit = agenttidy_core::cleanup::CleanupUnit {
            id: format!("{INSTALLATION_ID}:session:{id}"),
            provider: ProviderId::new(ProviderId::WORKBUDDY),
            installation_id: installation.id.clone(),
            kind: CleanupUnitKind::FileSet,
            resources: vec![],
            estimated_reclaimable_bytes: 0,
            fingerprint: ResourceFingerprint {
                path: transcript.clone(),
                identity: FileIdentity {
                    volume: 0,
                    index: 0,
                },
                size_bytes: 0,
                mtime_ms: 0,
            },
            locator: CleanupLocator::FileSet {
                paths: vec![transcript],
            },
            preconditions: WorkBuddyAdapter::fileset_preconditions(),
            operation: CleanupOperation {
                action: CleanupAction::Trash,
            },
            resource_kind: ResourceKind::Session,
        };
        let error = WorkBuddyAdapter
            .validate_cleanup_unit(&unit, &installation)
            .await
            .expect_err("missing row must reject the unit");
        assert!(
            error.to_string().contains("no longer maps"),
            "unexpected error: {error}"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
