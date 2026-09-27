//! Read-only Codex provider adapter (`Start.md` §10, Phase 3).
//!
//! The first implementation reads only known JSONL rollout files. Codex's
//! `state_5.sqlite` remains authoritative for its richer thread index, but
//! is intentionally not guessed at until its schema mapping has contract
//! coverage. Missing or unreadable index data therefore degrades diagnostics
//! instead of changing files or inventing session relationships.

use agenttidy_core::{
    AgentCapabilities, AgentInstallation, AgentSnapshot, CapabilityStatus, CapabilityTopic,
    CleanupPrecondition, CleanupUnit, InstallationStatus, ManagedBy, Ownership, Platform,
    ProjectRef, ProviderId, Resource, ResourceId, ResourceKind, ResourceLocator, ResourceRef,
    ScanOptions, ScanProblem, Session, SessionId, SessionLifecycle, SizeConfidence, SizeInfo,
};
use agenttidy_infrastructure::disk_usage::UsageAccumulator;
use agenttidy_infrastructure::fs_probe::{walk_tree, EntryKind, WalkProblem};
use agenttidy_infrastructure::processes::{any_process_running, ProcessSignature};
use agenttidy_infrastructure::sqlite::ReadOnlyDb;
use agenttidy_infrastructure::workspace_safety::inspect_workspace_safety;
use agenttidy_provider_api::{AgentProviderAdapter, ProviderInspection};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
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
        if let Some(root) = &state_root {
            if !root.join("state_5.sqlite").exists() {
                problems.insert(
                    "thread-index-missing".into(),
                    ScanProblem::Warning {
                        message: "state_5.sqlite missing; JSONL-only scan will be degraded".into(),
                    },
                );
            }
        }
        Ok(ProviderInspection {
            version: installation.version.clone(),
            schema_versions: BTreeMap::new(),
            journal_modes: BTreeMap::new(),
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

    /// Phase 6: deliberately returns no cleanup units.
    ///
    /// Two independent gates keep Codex cleanup out of v0.1-alpha:
    /// - `~/Documents/Codex/` is a user-mixed default workspace
    ///   (`Start.md` §2.3 — files already inside the user's project are
    ///   never default-cleanup candidates), and
    /// - trashing session rollout JSONLs would orphan the authoritative
    ///   `state_5.sqlite.threads` index rows, which needs a coordinated
    ///   `ProviderOperation` (Phase 7, per `Start.md` §19).
    async fn build_cleanup_units(
        &self,
        snapshot: &AgentSnapshot,
    ) -> anyhow::Result<Vec<CleanupUnit>> {
        let _ = snapshot;
        Ok(Vec::new())
    }

    /// Phase 6: no units are built, so no provider-specific preconditions
    /// apply yet. Phase 7 wires the §12.2 triggers (writer-lock absent,
    /// thread-index schema unchanged) alongside `build_cleanup_units`.
    async fn validate_cleanup_unit(
        &self,
        unit: &CleanupUnit,
        installation: &AgentInstallation,
    ) -> anyhow::Result<Vec<CleanupPrecondition>> {
        let _ = (unit, installation);
        Ok(Vec::new())
    }
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
}
