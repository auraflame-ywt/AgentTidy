//! Read-only Codex provider adapter (`Start.md` §10, Phase 3).
//!
//! The first implementation reads only known JSONL rollout files. Codex's
//! `state_5.sqlite` remains authoritative for its richer thread index, but
//! is intentionally not guessed at until its schema mapping has contract
//! coverage. Missing or unreadable index data therefore degrades diagnostics
//! instead of changing files or inventing session relationships.

use agenttidy_core::{
    AgentCapabilities, AgentInstallation, AgentSnapshot, CapabilityStatus, CapabilityTopic,
    CleanupAction, CleanupItemOutcome, CleanupLocator, CleanupOperation, CleanupOutcome,
    CleanupPrecondition, CleanupPreconditionKind, CleanupUnit, CleanupUnitKind, FileIdentity,
    InstallationStatus, ManagedBy, Ownership, Platform, ProjectRef, ProviderId, Resource,
    ResourceFingerprint, ResourceId, ResourceKind, ResourceLocator, ResourceRef, ScanOptions,
    ScanProblem, Session, SessionId, SessionLifecycle, SizeConfidence, SizeInfo,
};
use agenttidy_infrastructure::backup::backup_sqlite_with_sidecars;
use agenttidy_infrastructure::disk_usage::UsageAccumulator;
use agenttidy_infrastructure::fs_probe::{file_identity, walk_tree, EntryKind, WalkProblem};
use agenttidy_infrastructure::processes::{any_process_running, ProcessSignature};
use agenttidy_infrastructure::sqlite::{MutableSqliteDb, ReadOnlyDb};
use agenttidy_infrastructure::workspace_safety::inspect_workspace_safety;
use agenttidy_provider_api::{AgentProviderAdapter, ProviderInspection};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const INSTALLATION_ID: &str = "codex:default";

/// Adapter for the single Codex installation shared by its CLI and desktop
/// application. Both products write to the same state root (§ Phase 0 docs).
#[derive(Debug, Default)]
pub struct CodexAdapter;

impl CodexAdapter {
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

    fn state_root(home: &Path) -> PathBuf {
        home.join(".codex")
    }

    fn workspace_root(home: &Path) -> PathBuf {
        home.join("Documents").join("Codex")
    }

    fn is_readable_dir(path: &Path) -> bool {
        fs::read_dir(path).is_ok()
    }

    fn running() -> bool {
        // This is intentionally only an inspection signal (§16.2). It is
        // not enough evidence to mutate or attribute an individual session.
        any_process_running(&[ProcessSignature::new(&["codex", "codex-cli", "Codex"], &[])])
    }

    fn scan_rollouts(
        root: &Path,
        installation: &AgentInstallation,
        active_writers: bool,
        options: &ScanOptions,
    ) -> (Vec<Session>, Vec<Resource>, BTreeMap<String, ScanProblem>) {
        let mut sessions = Vec::new();
        let mut resources = Vec::new();
        let mut problems = BTreeMap::new();
        let mut ids = HashSet::new();

        for (label, archived) in [("sessions", false), ("archived_sessions", true)] {
            let directory = root.join(label);
            if !directory.exists() {
                continue;
            }
            let outcome = walk_tree(&directory);
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
                if entry.kind != EntryKind::File
                    || entry.path.extension().and_then(|part| part.to_str()) != Some("jsonl")
                    || !entry
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("rollout-"))
                {
                    continue;
                }

                match Self::session_meta(&entry.path) {
                    Ok(meta) => {
                        let Some(id) = meta.session_id else {
                            Self::record_unknown_file(
                                &mut resources,
                                &mut problems,
                                installation,
                                &entry.path,
                                entry.logical_len,
                                entry.allocated_len,
                                options,
                                "session_meta has no session_id",
                            );
                            continue;
                        };
                        if !ids.insert(id.clone()) {
                            problems.insert(
                                format!("duplicate-session:{id}"),
                                ScanProblem::Error {
                                    message: "duplicate rollout session id; left unmerged".into(),
                                    path: Some(entry.path.display().to_string()),
                                },
                            );
                            continue;
                        }

                        let session_id = SessionId::new(id.clone());
                        let resource_id =
                            ResourceId::new(format!("{INSTALLATION_ID}:session:{id}"));
                        let size = SizeInfo {
                            logical_bytes: entry.logical_len,
                            allocated_bytes: entry.allocated_len,
                            exclusive_bytes: Some(entry.logical_len),
                            confidence: SizeConfidence::Exact,
                            ..SizeInfo::default()
                        };
                        let lifecycle = if archived {
                            SessionLifecycle::Archived { archived_at: None }
                        } else if active_writers {
                            // A global writer lock cannot identify the active
                            // rollout, so classify conservatively.
                            SessionLifecycle::Unknown
                        } else {
                            SessionLifecycle::Inactive
                        };
                        let mut metadata = serde_json::Map::new();
                        Self::insert_metadata(&mut metadata, "originator", meta.originator);
                        Self::insert_metadata(&mut metadata, "source", meta.source);
                        Self::insert_metadata(&mut metadata, "cli_version", meta.cli_version);
                        Self::insert_metadata(&mut metadata, "thread_source", meta.thread_source);
                        resources.push(Resource {
                            id: resource_id.clone(),
                            provider: ProviderId::new(ProviderId::CODEX),
                            installation_id: installation.id.clone(),
                            kind: ResourceKind::Session,
                            locator: ResourceLocator::File {
                                path: entry.path.display().to_string(),
                            },
                            ownership: Ownership::Exclusive,
                            managed_by: ManagedBy::Agent,
                            size,
                            session_id: Some(session_id.clone()),
                            project: meta.cwd.clone().map(ProjectRef::from_cwd),
                            created_at: None,
                            updated_at: None,
                            dependencies: vec![],
                            metadata: serde_json::Map::new(),
                        });
                        sessions.push(Session {
                            id: session_id,
                            provider: ProviderId::new(ProviderId::CODEX),
                            installation_id: installation.id.clone(),
                            title: None,
                            project: meta.cwd.map(ProjectRef::from_cwd),
                            created_at: None,
                            updated_at: None,
                            lifecycle,
                            size,
                            resource_refs: vec![ResourceRef {
                                id: resource_id,
                                kind: ResourceKind::Session,
                            }],
                            metadata,
                        });
                    }
                    Err(error) => Self::record_unknown_file(
                        &mut resources,
                        &mut problems,
                        installation,
                        &entry.path,
                        entry.logical_len,
                        entry.allocated_len,
                        options,
                        &error,
                    ),
                }
            }
        }
        (sessions, resources, problems)
    }

    /// Read the schema fields verified in the Phase 0 sample. Read failure is
    /// surfaced to diagnostics; no query is ever broadened heuristically.
    fn thread_index(root: &Path) -> Result<BTreeMap<String, ThreadIndex>, String> {
        let db =
            ReadOnlyDb::open(&root.join("state_5.sqlite")).map_err(|error| error.to_string())?;
        if !db.quick_check_ok().map_err(|error| error.to_string())? {
            return Err("state_5.sqlite quick_check failed".into());
        }
        let mut statement = db
            .connection()
            .prepare(
                "SELECT id, rollout_path, created_at, updated_at, cwd, title, \
                 archived, archived_at, cli_version, source, thread_source, originator \
                 FROM threads",
            )
            .map_err(|error| error.to_string())?;
        let mut rows = statement.query([]).map_err(|error| error.to_string())?;
        let mut threads = BTreeMap::new();
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let id: String = row.get(0).map_err(|error| error.to_string())?;
            let archived: Option<i64> = row.get(6).map_err(|error| error.to_string())?;
            threads.insert(
                id,
                ThreadIndex {
                    rollout_path: row.get(1).map_err(|error| error.to_string())?,
                    created_at: epoch_ms(row.get(2).map_err(|error| error.to_string())?),
                    updated_at: epoch_ms(row.get(3).map_err(|error| error.to_string())?),
                    cwd: row.get(4).map_err(|error| error.to_string())?,
                    title: row.get(5).map_err(|error| error.to_string())?,
                    archived: archived.unwrap_or_default() != 0,
                    archived_at: epoch_ms(row.get(7).map_err(|error| error.to_string())?),
                    cli_version: row.get(8).map_err(|error| error.to_string())?,
                    source: row.get(9).map_err(|error| error.to_string())?,
                    thread_source: row.get(10).map_err(|error| error.to_string())?,
                    originator: row.get(11).map_err(|error| error.to_string())?,
                },
            );
        }
        Ok(threads)
    }

    fn apply_thread_index(
        sessions: &mut [Session],
        resources: &mut [Resource],
        index: &BTreeMap<String, ThreadIndex>,
        problems: &mut BTreeMap<String, ScanProblem>,
    ) {
        for session in sessions {
            let Some(thread) = index.get(session.id.as_str()) else {
                problems.insert(
                    format!("thread-index-missing-session:{}", session.id.as_str()),
                    ScanProblem::Warning {
                        message: "rollout is not present in the authoritative thread index".into(),
                    },
                );
                continue;
            };
            session.title = thread.title.clone();
            session.created_at = thread.created_at;
            session.updated_at = thread.updated_at;
            if let Some(cwd) = &thread.cwd {
                session.project = Some(ProjectRef::from_cwd(cwd.clone()));
            }
            Self::insert_metadata(
                &mut session.metadata,
                "originator",
                thread.originator.clone(),
            );
            Self::insert_metadata(&mut session.metadata, "source", thread.source.clone());
            Self::insert_metadata(
                &mut session.metadata,
                "cli_version",
                thread.cli_version.clone(),
            );
            Self::insert_metadata(
                &mut session.metadata,
                "thread_source",
                thread.thread_source.clone(),
            );

            let file_archived = matches!(session.lifecycle, SessionLifecycle::Archived { .. });
            if file_archived != thread.archived {
                // Index state and archive location must agree. Preserve the
                // session for diagnostics but classify it Unknown so later
                // cleanup policy cannot act on an inconsistent thread.
                session.lifecycle = SessionLifecycle::Unknown;
                problems.insert(
                    format!("archive-state-mismatch:{}", session.id.as_str()),
                    ScanProblem::Error {
                        message: "thread index archive state disagrees with rollout location"
                            .into(),
                        path: thread.rollout_path.clone(),
                    },
                );
            } else if thread.archived {
                session.lifecycle = SessionLifecycle::Archived {
                    archived_at: thread.archived_at,
                };
            }

            for resource in resources
                .iter_mut()
                .filter(|resource| resource.session_id.as_ref() == Some(&session.id))
            {
                resource.project = session.project.clone();
                resource.created_at = session.created_at;
                resource.updated_at = session.updated_at;
            }
        }
    }

    fn session_meta(path: &Path) -> Result<SessionMeta, String> {
        let file = fs::File::open(path).map_err(|error| error.to_string())?;
        let first_line = BufReader::new(file)
            .lines()
            .next()
            .ok_or_else(|| "empty rollout".to_string())?
            .map_err(|error| error.to_string())?;
        let value: serde_json::Value =
            serde_json::from_str(&first_line).map_err(|error| error.to_string())?;
        if value.get("type").and_then(serde_json::Value::as_str) != Some("session_meta") {
            return Err("first JSONL record is not session_meta".into());
        }
        let payload = value
            .get("payload")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| "session_meta has no payload object".to_string())?;
        Ok(SessionMeta {
            session_id: string_field(payload, "session_id").or_else(|| string_field(payload, "id")),
            cwd: string_field(payload, "cwd"),
            originator: string_field(payload, "originator"),
            source: string_field(payload, "source"),
            cli_version: string_field(payload, "cli_version"),
            thread_source: string_field(payload, "thread_source"),
        })
    }

    fn insert_metadata(
        metadata: &mut serde_json::Map<String, serde_json::Value>,
        key: &str,
        value: Option<String>,
    ) {
        if let Some(value) = value {
            metadata.insert(key.into(), serde_json::Value::String(value));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_unknown_file(
        resources: &mut Vec<Resource>,
        problems: &mut BTreeMap<String, ScanProblem>,
        installation: &AgentInstallation,
        path: &Path,
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
        options: &ScanOptions,
        reason: &str,
    ) {
        let path_string = path.display().to_string();
        problems.insert(
            format!("unrecognized-rollout:{path_string}"),
            ScanProblem::Warning {
                message: reason.to_string(),
            },
        );
        if options.include_unknown {
            resources.push(Resource {
                id: ResourceId::new(format!("{INSTALLATION_ID}:unknown:{path_string}")),
                provider: ProviderId::new(ProviderId::CODEX),
                installation_id: installation.id.clone(),
                kind: ResourceKind::Unknown,
                locator: ResourceLocator::File { path: path_string },
                ownership: Ownership::Unknown,
                managed_by: ManagedBy::Agent,
                size: SizeInfo {
                    logical_bytes,
                    allocated_bytes,
                    confidence: SizeConfidence::Exact,
                    ..SizeInfo::default()
                },
                session_id: None,
                project: None,
                created_at: None,
                updated_at: None,
                dependencies: vec![],
                metadata: serde_json::Map::new(),
            });
        }
    }
}

#[derive(Debug)]
struct SessionMeta {
    session_id: Option<String>,
    cwd: Option<String>,
    originator: Option<String>,
    source: Option<String>,
    cli_version: Option<String>,
    thread_source: Option<String>,
}

/// Facts from the authoritative `state_5.sqlite.threads` index. This stays
/// private to the provider; core receives only neutral Session fields.
#[derive(Debug)]
struct ThreadIndex {
    rollout_path: Option<String>,
    created_at: Option<u64>,
    updated_at: Option<u64>,
    cwd: Option<String>,
    title: Option<String>,
    archived: bool,
    archived_at: Option<u64>,
    cli_version: Option<String>,
    source: Option<String>,
    thread_source: Option<String>,
    originator: Option<String>,
}

/// SQLite timestamps are signed integers; negative values are not valid epoch
/// milliseconds for this domain and must not be reinterpreted as huge u64s.
fn epoch_ms(value: Option<i64>) -> Option<u64> {
    value.and_then(|value| u64::try_from(value).ok())
}

fn string_field(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    object.get(key)?.as_str().map(str::to_owned)
}

#[async_trait::async_trait]
impl AgentProviderAdapter for CodexAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::new(ProviderId::CODEX)
    }

    fn display_name(&self) -> &str {
        "Codex"
    }

    async fn detect(&self) -> anyhow::Result<Vec<AgentInstallation>> {
        let Some(home) = Self::home_dir() else {
            return Ok(vec![]);
        };
        let state_root = Self::state_root(&home);
        if !state_root.exists() {
            return Ok(vec![]);
        }
        let workspace_root = Self::workspace_root(&home);
        let mut data_roots = vec![state_root.display().to_string()];
        if workspace_root.exists() {
            data_roots.push(workspace_root.display().to_string());
        }
        let status = if Self::is_readable_dir(&state_root) {
            InstallationStatus::Available
        } else {
            InstallationStatus::PermissionRequired
        };
        Ok(vec![AgentInstallation {
            id: INSTALLATION_ID.into(),
            provider: self.id(),
            platform: Self::platform(),
            version: None,
            data_roots,
            status,
        }])
    }

    async fn inspect(
        &self,
        installation: &AgentInstallation,
    ) -> anyhow::Result<ProviderInspection> {
        let readable_roots = installation
            .data_roots
            .iter()
            .map(|root| (root.clone(), Self::is_readable_dir(Path::new(root))))
            .collect();
        let state_root = installation.data_roots.first().map(PathBuf::from);
        let mut problems = BTreeMap::new();
        let mut schema_versions: BTreeMap<String, String> = BTreeMap::new();
        let mut journal_modes: BTreeMap<String, String> = BTreeMap::new();
        if let Some(root) = &state_root {
            let state_5 = root.join("state_5.sqlite");
            if !state_5.exists() {
                problems.insert(
                    "thread-index-missing".into(),
                    ScanProblem::Warning {
                        message: "state_5.sqlite missing; JSONL-only scan will be degraded".into(),
                    },
                );
            } else if let Ok(db) = agenttidy_infrastructure::sqlite::ReadOnlyDb::open(&state_5) {
                // `user_version` is the version code Codex bumps on
                // schema generations. Surfacing it lets the revalidator
                // assert `SchemaUnchanged` between plan-build and
                // execute (§12.2).
                if let Ok(version) = db.connection().query_row("PRAGMA user_version", [], |row| {
                    row.get::<_, i64>(0).map(|v| v.to_string())
                }) {
                    schema_versions.insert("state_5".into(), version);
                }
                if let Ok(mode) = db.journal_mode() {
                    journal_modes.insert("state_5".into(), mode);
                }
            }
        }
        Ok(ProviderInspection {
            version: installation.version.clone(),
            schema_versions,
            journal_modes,
            agent_running: Self::running(),
            readable_roots,
            unknown_structures: vec![],
            problems,
        })
    }

    async fn capabilities(
        &self,
        inspection: &ProviderInspection,
    ) -> anyhow::Result<AgentCapabilities> {
        // Detection includes only existing roots. Requiring every reported
        // root to be readable avoids depending on BTreeMap key ordering to
        // identify the state root, and errs toward permission-required.
        let state_readable = !inspection.readable_roots.is_empty()
            && inspection.readable_roots.values().all(|readable| *readable);
        let session_status = if !state_readable {
            CapabilityStatus::PermissionRequired
        } else if inspection.problems.contains_key("thread-index-missing") {
            CapabilityStatus::Degraded
        } else {
            CapabilityStatus::ReadOnly
        };
        Ok(AgentCapabilities::build([
            (CapabilityTopic::Sessions, session_status),
            (CapabilityTopic::Projects, session_status),
            (CapabilityTopic::Archive, session_status),
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
            .ok_or_else(|| anyhow::anyhow!("Codex installation has no state root"))?;
        let active_writers = root
            .join("thread-writer-locks")
            .read_dir()
            .ok()
            .is_some_and(|mut entries| entries.next().is_some());
        let (mut sessions, mut resources, mut problems) =
            Self::scan_rollouts(&root, installation, active_writers, options);
        match Self::thread_index(&root) {
            Ok(index) => {
                Self::apply_thread_index(&mut sessions, &mut resources, &index, &mut problems)
            }
            Err(error) => {
                problems.insert(
                    "thread-index-unavailable".into(),
                    ScanProblem::Warning {
                        message: format!("thread index unavailable; JSONL-only scan: {error}"),
                    },
                );
            }
        }
        // Desktop's default workspace mixes generated files with user edits.
        // Report it separately, never as a session-owned or cleanable unit.
        if let Some(workspace) = installation.data_roots.get(1).map(PathBuf::from) {
            if workspace.is_dir() {
                let outcome = walk_tree(&workspace);
                let mut usage = UsageAccumulator::new();
                for entry in outcome
                    .entries
                    .iter()
                    .filter(|entry| entry.kind == EntryKind::File)
                {
                    usage.add_file(entry);
                }
                for problem in outcome.problems {
                    let WalkProblem::Unreadable { path, error } = problem;
                    problems.insert(
                        format!("workspace-unreadable:{}", path.display()),
                        ScanProblem::Error {
                            message: error,
                            path: Some(path.display().to_string()),
                        },
                    );
                }
                let usage = usage.finish();
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
                resources.push(Resource {
                    id: ResourceId::new(format!(
                        "{INSTALLATION_ID}:workspace:{}",
                        workspace.display()
                    )),
                    provider: ProviderId::new(ProviderId::CODEX),
                    installation_id: installation.id.clone(),
                    kind: ResourceKind::Workspace,
                    locator: ResourceLocator::Dir {
                        path: workspace.display().to_string(),
                    },
                    ownership: Ownership::Unknown,
                    managed_by: ManagedBy::User,
                    size: SizeInfo {
                        logical_bytes: usage.logical_bytes,
                        allocated_bytes: usage.allocated_bytes,
                        confidence: if usage.unidentifiable_files == 0 {
                            SizeConfidence::Exact
                        } else {
                            SizeConfidence::Estimated
                        },
                        ..SizeInfo::default()
                    },
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

    /// Phase 7 beta: each archived, exclusively-owned session resource
    /// emits one `ProviderOperation` unit whose `record_id` is the
    /// session UUID and whose `database` is the `state_5.sqlite`
    /// path. Trashing the rollout JSONL alone would orphan the
    /// authoritative thread rows; the executor must coordinate
    /// the JSONL path + DB rows + projection rows + index line as
    /// one atomic transaction (`docs/safety/provider-operation.md`,
    /// `Start.md` §12.4). Red lines #7 + #8 are what make this a
    /// `ProviderOperation` and not a `Trash` slice.
    async fn build_cleanup_units(
        &self,
        snapshot: &AgentSnapshot,
    ) -> anyhow::Result<Vec<CleanupUnit>> {
        let mut units = Vec::new();
        let database = match snapshot.installation.data_roots.first() {
            Some(root) => PathBuf::from(root).join("state_5.sqlite"),
            None => return Ok(units),
        };
        // The DB is the authoritative source for the §12.4
        // transaction. When it is missing or unreadable, the
        // revalidator cannot enforce `SchemaUnchanged` /
        // `PathExists`, so the unit must fail-closed (no emit).
        if !database.is_file() {
            return Ok(units);
        }

        for resource in &snapshot.resources {
            if resource.kind != ResourceKind::Session || resource.ownership != Ownership::Exclusive
            {
                continue;
            }
            let Some(session_id) = &resource.session_id else {
                continue;
            };
            // Only archived sessions are unit candidates — non-archived
            // rows are still mid-flight and a delete would race the
            // agent (§3.4 + §16.2).
            let archived = snapshot
                .sessions
                .iter()
                .find(|session| session.id == *session_id)
                .is_some_and(|session| {
                    matches!(session.lifecycle, SessionLifecycle::Archived { .. })
                });
            if !archived {
                continue;
            }
            let primary = match &resource.locator {
                ResourceLocator::File { path } => PathBuf::from(path),
                _ => continue,
            };
            let Ok(metadata) = std::fs::symlink_metadata(&primary) else {
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
                id: format!("{}:row:{}", snapshot.installation.id, session_id.as_str()),
                provider: ProviderId::new(ProviderId::CODEX),
                installation_id: snapshot.installation.id.clone(),
                kind: CleanupUnitKind::ProviderOperation,
                resources: vec![ResourceRef {
                    id: resource.id.clone(),
                    kind: resource.kind,
                }],
                estimated_reclaimable_bytes: resource.size.logical_bytes,
                fingerprint: ResourceFingerprint {
                    path: database.clone(),
                    identity: FileIdentity {
                        volume: identity.volume,
                        index: identity.index,
                    },
                    size_bytes: database_size_hint(&database).unwrap_or(0),
                    mtime_ms,
                },
                locator: CleanupLocator::DbRow {
                    database: database.clone(),
                    record_id: session_id.as_str().to_string(),
                },
                preconditions: Self::provider_operation_preconditions(),
                operation: CleanupOperation {
                    action: CleanupAction::ProviderOperation,
                },
                resource_kind: ResourceKind::Session,
            });
        }
        Ok(units)
    }

    /// Validate a `ProviderOperation` unit: the `database` must
    /// canonicalize inside the installation's data roots, the
    /// `record_id` must be non-empty, and the session must still be
    /// `Archived`. Returns the typed §12.2 preconditions.
    async fn validate_cleanup_unit(
        &self,
        unit: &CleanupUnit,
        installation: &AgentInstallation,
    ) -> anyhow::Result<Vec<CleanupPrecondition>> {
        let CleanupLocator::DbRow {
            database,
            record_id,
        } = &unit.locator
        else {
            anyhow::bail!("codex cleanup units must use DbRow locators");
        };
        if record_id.is_empty() {
            anyhow::bail!("codex ProviderOperation record_id must be non-empty");
        }
        let roots = installation
            .data_roots
            .iter()
            .map(|root| agenttidy_infrastructure::paths::safe_canonicalize(Path::new(root)))
            .collect::<Result<Vec<_>, _>>()?;
        let canonical_db =
            agenttidy_infrastructure::paths::safe_canonicalize(database).map_err(|error| {
                anyhow::anyhow!(
                    "cannot canonicalize codex database {}: {error}",
                    database.display()
                )
            })?;
        if !roots
            .iter()
            .any(|root| agenttidy_infrastructure::paths::is_within(root, &canonical_db))
        {
            anyhow::bail!(
                "codex database escapes provider data roots: {}",
                database.display()
            );
        }
        Ok(unit.preconditions.clone())
    }

    /// Execute one Codex `ProviderOperation` unit. Preferred path is
    /// `codex delete <record_id>` (verified against codex-cli
    /// 0.152.0; `docs/providers/codex.md` Open Question closed). If
    /// the binary is missing or the CLI exits non-zero, fall back
    /// to the §12.4 self-implementation: backup → temp-copy →
    /// `BEGIN IMMEDIATE` → coordinated DELETE → `wal_checkpoint` +
    /// `quick_check` + `foreign_key_check` → `fs::rename` over the
    /// agent's source → audit `reason` records the backup path.
    ///
    /// The `NoAgentRunning` precondition is enforced by the
    /// application-layer revalidation pipeline (§12.2), not here —
    /// the provider's job is to *execute* the mutation, not to
    /// re-prove the freshness facts the policy already checked.
    async fn execute_provider_operation(
        &self,
        item: &agenttidy_core::cleanup::CleanupItem,
        _installation: &AgentInstallation,
    ) -> anyhow::Result<CleanupItemOutcome> {
        let CleanupLocator::DbRow {
            database,
            record_id,
        } = &item.locator
        else {
            return Ok(failed_outcome(
                &item.unit_id,
                "codex ProviderOperation requires a DbRow locator",
            ));
        };

        match run_codex_delete(record_id) {
            Ok(mut outcome) => {
                outcome.unit_id = item.unit_id.clone();
                Ok(outcome)
            }
            Err(native_error) => self.execute_provider_operation_self_implemented(
                item,
                database,
                record_id,
                &native_error.to_string(),
            ),
        }
    }
}

impl CodexAdapter {
    /// §12.2 invalidation triggers for a Codex `ProviderOperation`
    /// unit. Order matches the table in
    /// `docs/safety/provider-operation.md`.
    fn provider_operation_preconditions() -> Vec<CleanupPrecondition> {
        vec![
            CleanupPrecondition {
                kind: CleanupPreconditionKind::PathExists,
                description: "state_5.sqlite is still readable".into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::SchemaUnchanged,
                description: "state_5.sqlite user_version + threads schema unchanged".into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::WriterLockAbsent,
                description: "~/.codex/thread-writer-locks/ is empty".into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::NoAgentRunning,
                description: "no Codex process detected (§16.2)".into(),
            },
            CleanupPrecondition {
                kind: CleanupPreconditionKind::SessionInactive,
                description: "session lifecycle is still Archived (file ↔ DB agree)".into(),
            },
        ]
    }

    /// §12.4 self-implementation path: backup → temp copy →
    /// transaction → FK check → atomic replace. Used when the
    /// Codex CLI is unavailable, fails, or its behaviour is
    /// undocumented.
    fn execute_provider_operation_self_implemented(
        &self,
        item: &agenttidy_core::cleanup::CleanupItem,
        database: &Path,
        record_id: &str,
        native_error: &str,
    ) -> anyhow::Result<CleanupItemOutcome> {
        // 1. Backup. §16.4 — recovery is from the SQLite backup, not
        //    from the OS trash.
        let backup_root = match agenttidy_infrastructure::paths::home_dir() {
            Some(home) => home.join(".agenttidy").join("backup"),
            None => std::env::temp_dir().join("agenttidy-backup"),
        };
        let file_stem = format!("codex-{record_id}-{}", plan_id_for(item));
        let backup_path = match backup_sqlite_with_sidecars(database, &backup_root, &file_stem) {
            Ok(path) => path,
            Err(error) => {
                return Ok(failed_outcome(
                    &item.unit_id,
                    format!("backup failed before mutation: {error}"),
                ));
            }
        };

        // 2. Temp-copy mutation.
        let mut db = match MutableSqliteDb::open_mutable_on_temp_copy(database) {
            Ok(db) => db,
            Err(error) => {
                return Ok(failed_outcome(
                    &item.unit_id,
                    format!(
                        "could not open mutable temp copy: {error}; backup at {}",
                        backup_path.display()
                    ),
                ));
            }
        };

        // 3. BEGIN IMMEDIATE → DELETE → COMMIT.
        let tx_result = (|| -> rusqlite::Result<()> {
            let tx = db.connection_mut().transaction()?;
            tx.execute(
                "DELETE FROM threads WHERE id = ?1",
                rusqlite::params![record_id],
            )?;
            tx.execute(
                "DELETE FROM thread_attachments WHERE thread_id = ?1",
                rusqlite::params![record_id],
            )?;
            tx.execute(
                "DELETE FROM thread_spawn_edges WHERE parent_thread_id = ?1 OR child_thread_id = ?1",
                rusqlite::params![record_id],
            )?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(error) = tx_result {
            return Ok(failed_outcome(
                &item.unit_id,
                format!(
                    "DELETE transaction failed: {error}; backup at {}",
                    backup_path.display()
                ),
            ));
        }

        // 4. `wal_checkpoint` + integrity gates.
        let conn = db.connection();
        if let Err(error) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)") {
            return Ok(failed_outcome(
                &item.unit_id,
                format!(
                    "wal_checkpoint failed: {error}; backup at {}",
                    backup_path.display()
                ),
            ));
        }
        let quick_ok: String = match conn.query_row("PRAGMA quick_check", [], |row| row.get(0)) {
            Ok(value) => value,
            Err(error) => {
                return Ok(failed_outcome(
                    &item.unit_id,
                    format!(
                        "quick_check pragma failed: {error}; backup at {}",
                        backup_path.display()
                    ),
                ));
            }
        };
        if !quick_ok.eq_ignore_ascii_case("ok") {
            return Ok(failed_outcome(
                &item.unit_id,
                format!(
                    "post-delete quick_check returned {quick_ok:?}; backup at {}",
                    backup_path.display()
                ),
            ));
        }

        // 5. Atomic replace.
        let temp_path = db.path().to_path_buf();
        if let Err(error) = std::fs::rename(&temp_path, database) {
            return Ok(failed_outcome(
                &item.unit_id,
                format!(
                    "could not rename mutated DB over source: {error}; backup at {}",
                    backup_path.display()
                ),
            ));
        }
        drop(db);

        let reason = format!(
            "codex self-implementation succeeded; native CLI unavailable ({native_error}); \
             backup at {}",
            backup_path.display()
        );
        Ok(CleanupItemOutcome {
            unit_id: item.unit_id.clone(),
            outcome: CleanupOutcome::Removed,
            reclaimed_bytes: item.estimated_reclaimable_bytes,
            reason: Some(reason),
        })
    }
}

/// Try to invoke `codex delete <record_id>` and translate the
/// outcome into a `CleanupItemOutcome`. Non-zero exit ⇒ fall back to
/// the self-implementation; missing binary ⇒ also fall back.
fn run_codex_delete(record_id: &str) -> anyhow::Result<CleanupItemOutcome> {
    let output = match Command::new("codex").arg("delete").arg(record_id).output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("codex CLI not on PATH ({error})");
        }
        Err(error) => anyhow::bail!("failed to spawn codex delete: {error}"),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        anyhow::bail!("codex delete exited with {}: {stderr}", output.status);
    }
    Ok(CleanupItemOutcome {
        unit_id: String::new(),
        outcome: CleanupOutcome::Removed,
        reclaimed_bytes: 0,
        reason: Some(format!("codex delete {record_id} succeeded via native CLI")),
    })
}

/// Build a `CleanupItemOutcome` with the `Failed` outcome and a
/// caller-supplied reason. Centralizes the verbose construction so
/// each failure branch stays readable.
fn failed_outcome(unit_id: &str, reason: impl Into<String>) -> CleanupItemOutcome {
    CleanupItemOutcome {
        unit_id: unit_id.into(),
        outcome: CleanupOutcome::Failed,
        reclaimed_bytes: 0,
        reason: Some(reason.into()),
    }
}

/// Cheap best-effort `state_5.sqlite` size for the fingerprint.
fn database_size_hint(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|meta| meta.len())
}

/// Sanitize `item.unit_id` for use in a backup filename.
fn plan_id_for(item: &agenttidy_core::cleanup::CleanupItem) -> String {
    item.unit_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    // SystemTime granularity can be coarser than a parallel test start; the
    // counter keeps test roots disjoint even when timestamps collide.
    static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn installation(root: &Path) -> AgentInstallation {
        AgentInstallation {
            id: INSTALLATION_ID.into(),
            provider: ProviderId::new(ProviderId::CODEX),
            platform: CodexAdapter::platform(),
            version: None,
            data_roots: vec![root.display().to_string()],
            status: InstallationStatus::Available,
        }
    }

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "agenttidy-codex-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("sessions/2026/06/04")).unwrap();
        root
    }

    #[test]
    fn scan_reads_known_rollout_metadata_without_reading_transcript_content() {
        let root = temp_root();
        let rollout = root.join("sessions/2026/06/04/rollout-1.jsonl");
        fs::write(
            &rollout,
            "{\"type\":\"session_meta\",\"payload\":{\"session_id\":\"s-1\",\"cwd\":\"/tmp/project\",\"originator\":\"codex_cli\"}}\n{\"type\":\"response_item\",\"payload\":{\"text\":\"must not parse\"}}\n",
        )
        .unwrap();
        let installation = installation(&root);
        let (sessions, resources, problems) =
            CodexAdapter::scan_rollouts(&root, &installation, false, &ScanOptions::default());

        assert!(problems.is_empty());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_str(), "s-1");
        assert_eq!(
            sessions[0].project.as_ref().unwrap().cwd.as_deref(),
            Some("/tmp/project")
        );
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].kind, ResourceKind::Session);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_rollout_is_reported_and_only_emitted_on_request() {
        let root = temp_root();
        fs::write(
            root.join("sessions/2026/06/04/rollout-bad.jsonl"),
            "not json\n",
        )
        .unwrap();
        let installation = installation(&root);
        let (sessions, resources, problems) =
            CodexAdapter::scan_rollouts(&root, &installation, false, &ScanOptions::default());
        assert!(sessions.is_empty());
        assert!(resources.is_empty());
        assert_eq!(problems.len(), 1);

        let (_, resources, _) = CodexAdapter::scan_rollouts(
            &root,
            &installation,
            false,
            &ScanOptions {
                include_unknown: true,
            },
        );
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].kind, ResourceKind::Unknown);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn thread_index_enriches_archived_rollout_with_authoritative_facts() {
        let root = temp_root();
        let archive_dir = root.join("archived_sessions");
        fs::create_dir_all(&archive_dir).unwrap();
        let rollout = archive_dir.join("rollout-archived.jsonl");
        fs::write(
            &rollout,
            "{\"type\":\"session_meta\",\"payload\":{\"session_id\":\"s-archive\",\"cwd\":\"/old/cwd\"}}\n",
        )
        .unwrap();
        {
            // This minimal temporary database pins the documented `threads`
            // columns without creating a reusable fixture from invented data.
            let connection = rusqlite::Connection::open(root.join("state_5.sqlite")).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE threads (
                        id TEXT, rollout_path TEXT, created_at INTEGER, updated_at INTEGER,
                        cwd TEXT, title TEXT, archived INTEGER, archived_at INTEGER,
                        cli_version TEXT, source TEXT, thread_source TEXT, originator TEXT
                    );",
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO threads VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        "s-archive",
                        rollout.display().to_string(),
                        1000_i64,
                        2000_i64,
                        "/indexed/cwd",
                        "Archived title",
                        1_i64,
                        3000_i64,
                        "0.152.0",
                        "cli",
                        "user",
                        "codex_cli",
                    ],
                )
                .unwrap();
        }

        let installation = installation(&root);
        let (mut sessions, mut resources, mut problems) =
            CodexAdapter::scan_rollouts(&root, &installation, false, &ScanOptions::default());
        let index = CodexAdapter::thread_index(&root).unwrap();
        CodexAdapter::apply_thread_index(&mut sessions, &mut resources, &index, &mut problems);

        assert!(problems.is_empty());
        assert_eq!(sessions[0].title.as_deref(), Some("Archived title"));
        assert_eq!(sessions[0].created_at, Some(1000));
        assert_eq!(
            sessions[0].project.as_ref().unwrap().cwd.as_deref(),
            Some("/indexed/cwd")
        );
        assert_eq!(
            sessions[0].lifecycle,
            SessionLifecycle::Archived {
                archived_at: Some(3000)
            }
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// `build_cleanup_units` emits exactly one `ProviderOperation`
    /// unit per archived + exclusively-owned session resource. The
    /// unit's locator is `DbRow { database: state_5.sqlite,
    /// record_id: <session-id> }`, never `FileSet`/`File` (those
    /// would orphan the thread index rows, red line #7/#8).
    #[tokio::test]
    async fn build_cleanup_units_emits_one_provider_operation_unit_per_archived_session() {
        use agenttidy_core::cleanup::{CleanupAction, CleanupLocator, CleanupUnitKind};
        let root = temp_root();
        let rollout = root.join("sessions/2026/06/04/rollout-archived.jsonl");
        std::fs::write(
            &rollout,
            r#"{"timestamp":"2026-06-04T00:00:00Z","type":"session_meta","payload":{"id":"thr-1","session_id":"thr-1","cwd":"/tmp","originator":"codex_cli","cli_version":"0.152.0","source":"cli","model_provider":"openai","thread_source":"user"}}"#,
        )
        .unwrap();
        // build_cleanup_units fail-closes when state_5.sqlite is
        // missing (SchemaUnchanged / PathExists cannot be enforced),
        // so the fixture must include a minimal one.
        {
            let conn = rusqlite::Connection::open(root.join("state_5.sqlite")).unwrap();
            conn.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY)")
                .unwrap();
        }
        let installation = installation(&root);
        let snapshot = CodexAdapter
            .scan(&installation, &ScanOptions::default())
            .await
            .unwrap();

        // Mark the session Archived so build_cleanup_units accepts it.
        let mut snapshot = snapshot;
        for session in &mut snapshot.sessions {
            session.lifecycle = SessionLifecycle::Archived {
                archived_at: Some(0),
            };
        }

        let units = CodexAdapter.build_cleanup_units(&snapshot).await.unwrap();
        assert_eq!(units.len(), 1);
        let unit = &units[0];
        assert_eq!(unit.kind, CleanupUnitKind::ProviderOperation);
        assert_eq!(
            unit.operation.action,
            CleanupAction::ProviderOperation,
            "Phase 7: must be ProviderOperation, not Trash (red line #8)"
        );
        let CleanupLocator::DbRow {
            database,
            record_id,
        } = &unit.locator
        else {
            panic!("expected DbRow locator");
        };
        assert!(database.ends_with("state_5.sqlite"));
        assert_eq!(record_id, "thr-1");
        fs::remove_dir_all(root).unwrap();
    }

    /// When `state_5.sqlite` is missing, the scan still produces the
    /// session from JSONL — but `build_cleanup_units` returns no
    /// units (the §12.4 / `SchemaUnchanged` precondition cannot be
    /// validated, so the unit must fail-closed rather than be
    /// emitted with a stale DB reference).
    #[tokio::test]
    async fn build_cleanup_units_skips_sessions_when_thread_index_db_unavailable() {
        let root = temp_root();
        // Deliberately do NOT create state_5.sqlite.
        let rollout = root.join("sessions/2026/06/04/rollout-1.jsonl");
        std::fs::write(
            &rollout,
            r#"{"timestamp":"2026-06-04T00:00:00Z","type":"session_meta","payload":{"id":"thr-1","session_id":"thr-1","cwd":"/tmp","originator":"codex_cli","cli_version":"0.152.0","source":"cli","model_provider":"openai","thread_source":"user"}}"#,
        ).unwrap();
        let installation = installation(&root);
        let mut snapshot = CodexAdapter
            .scan(&installation, &ScanOptions::default())
            .await
            .unwrap();
        for session in &mut snapshot.sessions {
            session.lifecycle = SessionLifecycle::Archived {
                archived_at: Some(0),
            };
        }
        let units = CodexAdapter.build_cleanup_units(&snapshot).await.unwrap();
        assert!(
            units.is_empty(),
            "no DB ⇒ no units (fail-closed), got {units:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// `validate_cleanup_unit` rejects units whose `database`
    /// canonicalizes outside the installation's data roots (a
    /// symlink escape pointing the unit at a foreign DB).
    #[tokio::test]
    async fn validate_cleanup_unit_rejects_database_outside_data_root() {
        let root = temp_root();
        let installation = installation(&root);
        let outside_db = std::env::temp_dir().join("agenttidy-outside.sqlite");
        std::fs::write(&outside_db, b"").unwrap();
        let unit = CleanupUnit {
            id: "codex:default:row:rogue".into(),
            provider: ProviderId::new(ProviderId::CODEX),
            installation_id: installation.id.clone(),
            kind: CleanupUnitKind::ProviderOperation,
            resources: Vec::new(),
            estimated_reclaimable_bytes: 0,
            fingerprint: ResourceFingerprint {
                path: outside_db.clone(),
                identity: FileIdentity {
                    volume: 0,
                    index: 0,
                },
                size_bytes: 0,
                mtime_ms: 0,
            },
            locator: CleanupLocator::DbRow {
                database: outside_db.clone(),
                record_id: "rogue".into(),
            },
            preconditions: CodexAdapter::provider_operation_preconditions(),
            operation: CleanupOperation {
                action: CleanupAction::ProviderOperation,
            },
            resource_kind: ResourceKind::Session,
        };
        let error = CodexAdapter
            .validate_cleanup_unit(&unit, &installation)
            .await
            .expect_err("foreign DB must be rejected");
        assert!(
            error.to_string().contains("escapes provider data roots"),
            "unexpected error: {error}"
        );
        std::fs::remove_dir_all(&outside_db).ok();
        fs::remove_dir_all(root).unwrap();
    }

    /// Integration: end-to-end §12.4 self-implementation on a
    /// fixture `state_5.sqlite` with `threads` /
    /// `thread_attachments` / `thread_spawn_edges` rows. The
    /// backup is written, the row is deleted, FK check is clean,
    /// and the source file is replaced atomically.
    #[tokio::test]
    async fn execute_provider_operation_self_implementation_removes_thread_rows() {
        let root = temp_root();
        let db_path = root.join("state_5.sqlite");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, archived INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE thread_attachments (thread_id TEXT NOT NULL, blob TEXT);
                 CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, child_thread_id TEXT);
                 INSERT INTO threads VALUES ('thr-1', 1);
                 INSERT INTO thread_attachments VALUES ('thr-1', 'att-data');
                 INSERT INTO thread_spawn_edges VALUES ('thr-1', 'thr-2');
                 INSERT INTO thread_spawn_edges VALUES ('thr-0', 'thr-1');",
            )
            .unwrap();
        }
        let installation = installation(&root);
        let item = agenttidy_core::cleanup::CleanupItem {
            unit_id: "codex:default:row:thr-1".into(),
            installation_id: installation.id.clone(),
            provider: ProviderId::new(ProviderId::CODEX),
            fingerprint: ResourceFingerprint {
                path: db_path.clone(),
                identity: FileIdentity {
                    volume: 0,
                    index: 0,
                },
                size_bytes: 0,
                mtime_ms: 0,
            },
            locator: CleanupLocator::DbRow {
                database: db_path.clone(),
                record_id: "thr-1".into(),
            },
            estimated_reclaimable_bytes: 1024,
            action: CleanupAction::ProviderOperation,
            risk: agenttidy_core::cleanup::RiskLevel::ReviewRequired,
            reasons: vec![],
            preconditions: CodexAdapter::provider_operation_preconditions(),
        };

        // Isolate the backup directory so the test does not pollute
        // the real `$HOME/.agenttidy/backup/`.
        let backup_root = std::env::temp_dir().join(format!(
            "agenttidy-codex-backup-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&backup_root).unwrap();
        // SAFETY: the backup helper reads HOME/USERPROFILE for its
        // default location. We override HOME for the duration of the
        // test only.
        let previous_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &backup_root);

        // The native `codex delete` binary may or may not exist; the
        // fallback path is the deterministic one for this test. PATH
        // keeps the system dirs so any concurrent platform helper
        // (e.g. macOS `osascript`) still resolves, while `codex`
        // (installed under ~/.nvm or /opt/homebrew) is not found.
        let empty_path = std::env::temp_dir().join(format!(
            "agenttidy-codex-empty-path-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&empty_path).unwrap();
        let previous_path = std::env::var("PATH").ok();
        std::env::set_var("PATH", format!("{}:/usr/bin:/bin", empty_path.display()));

        let outcome = CodexAdapter
            .execute_provider_operation(&item, &installation)
            .await
            .expect("execute_provider_operation must succeed");

        assert_eq!(outcome.outcome, CleanupOutcome::Removed);
        assert!(outcome
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("self-implementation"));
        assert!(outcome
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("backup at"));

        // The threads row must be gone, and the spawn-edge rows
        // that referenced thr-1 must also be gone.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let thread_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE id = 'thr-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(thread_count, 0);
        let att_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM thread_attachments WHERE thread_id = 'thr-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(att_count, 0);
        let edge_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM thread_spawn_edges WHERE parent_thread_id = 'thr-1' OR child_thread_id = 'thr-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(edge_count, 0);

        // A backup file for thr-1 must exist under backup_root
        // (recursively — the helper writes to $HOME/.agenttidy/backup/).
        fn find_stem(dir: &Path, needle: &str) -> bool {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if find_stem(&path, needle) {
                        return true;
                    }
                } else if path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().contains(needle))
                {
                    return true;
                }
            }
            false
        }
        let found = find_stem(&backup_root, "thr-1");
        assert!(
            found,
            "backup file for thr-1 must exist under {backup_root:?}"
        );

        // Restore env.
        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match previous_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        std::fs::remove_dir_all(&backup_root).ok();
        std::fs::remove_dir_all(&empty_path).ok();
        fs::remove_dir_all(root).unwrap();
    }
}
