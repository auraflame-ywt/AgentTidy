//! Antigravity metadata index adapter (`Start.md` §10, §18, §22).
//!
//! The macOS conversation index is verified; protobuf bodies are opaque.
//! Brain artifacts are measured without opening their contents. No cleanup,
//! archive or lifecycle inference is enabled by an idle index status.

use agenttidy_core::*;
use agenttidy_infrastructure::{
    disk_usage::UsageAccumulator,
    fs_probe::{walk_tree, EntryKind},
    paths::host_platform,
    processes::{any_process_running, ProcessSignature},
    sqlite::ReadOnlyDb,
};
use agenttidy_provider_api::{AgentProviderAdapter, ProviderInspection};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Default)]
pub struct AntigravityAdapter;

mod cleanup;
mod state;
const INDEX: &str = "conversation_summaries.db";
// Explicit projection avoids preview/raw_summary/FTS message bodies (§18).
const QUERY: &str = "SELECT conversation_id, title, last_modified_time, workspace_uris, status, parent_conversation_id, not_fully_idle FROM conversation_summaries ORDER BY conversation_id";

fn epoch(value: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(&value.replacen(' ', "T", 1))
        .ok()
        .and_then(|d| u64::try_from(d.timestamp_millis()).ok())
}

/// Database ids must remain single path components before joining any root.
fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn regular(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
}

impl AntigravityAdapter {
    fn running() -> bool {
        any_process_running(&[ProcessSignature::new(
            // The executable stem matches on both platforms: macOS ships
            // `Antigravity`/`Antigravity IDE`, Windows ships
            // `Antigravity.exe` (the extension is ignored by the
            // matcher).
            &["Antigravity", "Antigravity IDE", "language_server"],
            // The macOS bundle path and the Windows install directory
            // both identify the agent when the executable name is
            // generic (`Electron`-style hosts).
            &[
                "Antigravity.app",
                "Antigravity IDE.app",
                "programs/antigravity",
                "programs/antigravity ide",
            ],
        )])
    }

    /// Report the observed state root once, irrespective of installed app bundles.
    ///
    /// The state root is the same on both platforms: `~/.gemini/antigravity`.
    /// Verified on macOS (Antigravity 2.24.0 / IDE 2.5.5) and on Windows
    /// (same directory tree: `conversation_summaries.db`, `conversations/<UUID>.pb`,
    /// `brain/<UUID>/...`).
    fn discover() -> Option<PathBuf> {
        agenttidy_infrastructure::paths::home_dir().map(|h| h.join(".gemini/antigravity"))
    }
    fn root(installation: &AgentInstallation) -> anyhow::Result<&Path> {
        let root = Path::new(
            installation
                .data_roots
                .first()
                .ok_or_else(|| anyhow::anyhow!("missing Antigravity root"))?,
        );
        anyhow::ensure!(
            fs::symlink_metadata(root).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()),
            "Antigravity root unavailable or linked"
        );
        Ok(root)
    }
    fn open(root: &Path) -> anyhow::Result<ReadOnlyDb> {
        let path = root.join(INDEX);
        anyhow::ensure!(
            regular(&path),
            "Antigravity conversation index unavailable or linked"
        );
        for suffix in ["-wal", "-shm"] {
            let sidecar = root.join(format!("{INDEX}{suffix}"));
            anyhow::ensure!(
                !fs::symlink_metadata(&sidecar).is_ok_and(|m| m.file_type().is_symlink()),
                "linked Antigravity database sidecar"
            );
        }
        let db = ReadOnlyDb::open(&path)?;
        // Prepare the verified projection to fail closed on incompatible schemas.
        db.connection().prepare(QUERY)?;
        Ok(db)
    }
    fn scan_root(
        installation: &AgentInstallation,
        options: &ScanOptions,
    ) -> anyhow::Result<AgentSnapshot> {
        let root = Self::root(installation)?;
        let mut snapshot = AgentSnapshot {
            installation: installation.clone(),
            sessions: vec![],
            resources: vec![],
            problems: BTreeMap::new(),
            completed_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };
        let db = match Self::open(root) {
            Ok(db) => db,
            Err(error) => {
                snapshot.problems.insert(
                    "antigravity-index-unavailable".into(),
                    ScanProblem::Error {
                        message: format!("Antigravity index unavailable: {error}"),
                        path: Some(root.join(INDEX).display().to_string()),
                    },
                );
                return Ok(snapshot);
            }
        };
        let _running = Self::running();
        let safe_directory = |name: &str| {
            fs::symlink_metadata(root.join(name))
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        };
        let mut statement = db.connection().prepare(QUERY)?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, bool>(6)?,
            ))
        })?;
        for row in rows {
            let (id, title, time, workspace_json, status, parent, _not_idle) = match row {
                Ok(v) => v,
                Err(error) => {
                    snapshot.problems.insert(
                        "antigravity-index-row".into(),
                        ScanProblem::Error {
                            message: format!("Unreadable index row: {error}"),
                            path: None,
                        },
                    );
                    continue;
                }
            };
            if !valid_id(&id) {
                snapshot.problems.insert(
                    format!("antigravity-invalid-id:{id}"),
                    ScanProblem::Warning {
                        message: "Unsupported conversation id".into(),
                    },
                );
                continue;
            }
            let workspaces: Vec<String> = match serde_json::from_str(&workspace_json) {
                Ok(v) => v,
                Err(_) => {
                    snapshot.problems.insert(
                        format!("antigravity-workspaces:{id}"),
                        ScanProblem::Warning {
                            message: "Unsupported workspace URI metadata".into(),
                        },
                    );
                    vec![]
                }
            };
            let project = if workspaces.len() == 1 {
                url::Url::parse(&workspaces[0])
                    .ok()
                    .and_then(|u| u.to_file_path().ok())
                    .map(|p| ProjectRef::from_cwd(p.display().to_string()))
            } else {
                None
            };
            let annotation_path = root.join("annotations").join(format!("{id}.pbtxt"));
            let annotation = if !annotation_path.try_exists().unwrap_or(true) {
                Ok(state::Annotation::default())
            } else if safe_directory("annotations") {
                state::annotation(&annotation_path)
            } else {
                Err(anyhow::anyhow!(
                    "annotations directory is linked or unreadable"
                ))
            };
            if let Err(error) = &annotation {
                snapshot.problems.insert(
                    format!("antigravity-annotation:{id}"),
                    ScanProblem::Warning {
                        message: format!("Unreadable archive annotations: {error}"),
                    },
                );
            }
            let bodies: Vec<_> = ["pb", "db"]
                .into_iter()
                .map(|ext| root.join("conversations").join(format!("{id}.{ext}")))
                .filter(|p| regular(p))
                .collect();
            let body_verified = safe_directory("conversations")
                && bodies.len() == 1
                && match bodies[0].extension().and_then(|s| s.to_str()) {
                    Some("pb") => state::legacy_owned(&bodies[0], &id).unwrap_or(false),
                    Some("db") => state::sqlite_owned(&bodies[0], &id).unwrap_or(false),
                    _ => false,
                };
            // `CASCADE_RUN_STATUS_IDLE` proves only that the index writer is
            // idle — it cannot prove the session is cleanable (§18, docs/
            // providers/antigravity.md). Lifecycle therefore stays Unknown
            // for every row; archived/idle inference is not enabled.
            let lifecycle = SessionLifecycle::Unknown;
            let title = annotation
                .as_ref()
                .ok()
                .and_then(|a| a.title.clone())
                .filter(|t| !t.is_empty())
                .or_else(|| (!title.is_empty()).then_some(title));
            let mut metadata = serde_json::Map::new();
            metadata.insert("workspace_uris".into(), serde_json::json!(workspaces));
            metadata.insert("index_status".into(), status.into());
            metadata.insert("parent_conversation_id".into(), parent.into());
            metadata.insert("body_verified".into(), body_verified.into());
            snapshot.sessions.push(Session {
                id: SessionId::new(id),
                provider: installation.provider.clone(),
                installation_id: installation.id.clone(),
                title,
                project,
                created_at: None,
                updated_at: epoch(&time),
                lifecycle,
                size: SizeInfo::exact(0),
                resource_refs: vec![],
                metadata,
            });
        }
        let parents: HashSet<_> = snapshot
            .sessions
            .iter()
            .filter_map(|s| {
                s.metadata["parent_conversation_id"]
                    .as_str()
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
            })
            .collect();
        for session in &mut snapshot.sessions {
            session.metadata.insert(
                "has_session_dependents".into(),
                parents.contains(&session.id.0).into(),
            );
        }
        let ids: HashSet<_> = snapshot.sessions.iter().map(|s| s.id.0.clone()).collect();
        let mut seen = HashSet::new();
        for label in ["conversations", "annotations", "brain"] {
            let directory = root.join(label);
            if !safe_directory(label) {
                if directory.try_exists().unwrap_or(true) || label == "conversations" {
                    snapshot.problems.insert(
                        format!("antigravity-directory:{}", directory.display()),
                        ScanProblem::Warning {
                            message: "Antigravity resource directory missing, unreadable or linked"
                                .into(),
                        },
                    );
                }
                continue;
            }
            let walk = walk_tree(&directory);
            for problem in walk.problems {
                snapshot.problems.insert(
                    format!("antigravity-walk:{problem:?}"),
                    ScanProblem::Warning {
                        message: format!("Incomplete resource enumeration: {problem:?}"),
                    },
                );
            }
            for entry in walk.entries {
                if entry.kind != EntryKind::File {
                    continue;
                }
                let relative = entry.path.strip_prefix(&directory)?;
                let name = relative.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let candidate = if label == "brain" {
                    relative
                        .components()
                        .next()
                        .and_then(|p| p.as_os_str().to_str())
                } else if relative.components().count() == 1 {
                    if label == "annotations" {
                        name.strip_suffix(".pbtxt")
                    } else {
                        [".pb", ".db", ".db-wal", ".db-shm"]
                            .into_iter()
                            .find_map(|suffix| name.strip_suffix(suffix))
                    }
                } else {
                    None
                };
                let session_id = candidate.filter(|id| ids.contains(*id)).map(SessionId::new);
                if session_id.is_none() {
                    snapshot.problems.entry("antigravity-unindexed-files".into()).or_insert_with(||ScanProblem::Warning{message:"Files outside indexed sessions exist; enable include_unknown to measure them".into()});
                    if !options.include_unknown {
                        continue;
                    }
                }
                if let Some(identity) = entry.identity {
                    if !seen.insert(identity) {
                        continue;
                    }
                }
                let mut usage = UsageAccumulator::new();
                usage.add_file(&entry);
                let usage = usage.finish();
                let size = SizeInfo {
                    logical_bytes: usage.logical_bytes,
                    allocated_bytes: usage.allocated_bytes,
                    confidence: if usage.unidentifiable_files == 0 {
                        SizeConfidence::Exact
                    } else {
                        SizeConfidence::Estimated
                    },
                    ..SizeInfo::default()
                };
                let resource_id =
                    ResourceId::new(format!("{}:file:{}", installation.id, entry.path.display()));
                let session = session_id
                    .as_ref()
                    .and_then(|id| snapshot.sessions.iter_mut().find(|s| s.id == *id));
                let project = session.as_ref().and_then(|s| s.project.clone());
                let updated_at = session.as_ref().and_then(|s| s.updated_at);
                let owned = label != "brain"
                    && agenttidy_infrastructure::fs_probe::is_single_link_file(&entry.path)
                    && session.as_ref().is_some_and(|s| {
                        s.metadata["body_verified"] == true
                            && s.metadata["parent_conversation_id"] == ""
                            && s.metadata["has_session_dependents"] == false
                    });
                let kind = if session_id.is_none() {
                    ResourceKind::Unknown
                } else if label == "brain" {
                    ResourceKind::GeneratedFile
                } else {
                    ResourceKind::Session
                };
                if let Some(session) = session {
                    session.size = session.size.merge(size);
                    session.resource_refs.push(ResourceRef {
                        id: resource_id.clone(),
                        kind,
                    });
                }
                snapshot.resources.push(Resource {
                    id: resource_id,
                    provider: installation.provider.clone(),
                    installation_id: installation.id.clone(),
                    kind,
                    locator: ResourceLocator::File {
                        path: entry.path.display().to_string(),
                    },
                    ownership: if owned {
                        Ownership::Exclusive
                    } else {
                        Ownership::Unknown
                    },
                    managed_by: ManagedBy::Agent,
                    size,
                    session_id,
                    project,
                    created_at: None,
                    updated_at,
                    dependencies: vec![],
                    metadata: serde_json::Map::new(),
                });
            }
        }
        for name in [
            INDEX,
            "conversation_summaries.db-wal",
            "conversation_summaries.db-shm",
            "agyhub_summaries_proto.pb",
        ] {
            let path = root.join(name);
            if !regular(&path) {
                continue;
            }
            snapshot.resources.push(Resource {
                id: ResourceId::new(format!("{}:index:{name}", installation.id)),
                provider: installation.provider.clone(),
                installation_id: installation.id.clone(),
                kind: ResourceKind::Database,
                locator: ResourceLocator::File {
                    path: path.display().to_string(),
                },
                ownership: Ownership::Shared,
                managed_by: ManagedBy::Agent,
                size: SizeInfo::exact(fs::metadata(&path)?.len()),
                session_id: None,
                project: None,
                created_at: None,
                updated_at: None,
                dependencies: vec![],
                metadata: serde_json::Map::new(),
            });
        }
        for session in &mut snapshot.sessions {
            let has_body=snapshot.resources.iter().any(|r|r.session_id.as_ref()==Some(&session.id)&&matches!(&r.locator,ResourceLocator::File{path} if Path::new(path).extension().is_some_and(|e|e=="pb"||e=="db")));
            if !has_body {
                snapshot.problems.insert(format!("antigravity-transcript-missing:{}",session.id.0),ScanProblem::Warning{message:"Conversation index has no readable matching protobuf or SQLite transcript".into()});
                session.size.confidence = SizeConfidence::Estimated;
            }
        }
        if !snapshot.problems.is_empty() {
            for resource in &mut snapshot.resources {
                resource.size.confidence = SizeConfidence::Estimated;
            }
            for session in &mut snapshot.sessions {
                session.size.confidence = SizeConfidence::Estimated;
            }
        }
        Ok(snapshot)
    }
}

#[async_trait::async_trait]
impl AgentProviderAdapter for AntigravityAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::new(ProviderId::ANTIGRAVITY)
    }
    fn display_name(&self) -> &str {
        "Antigravity"
    }
    async fn detect(&self) -> anyhow::Result<Vec<AgentInstallation>> {
        let Some(root) = Self::discover() else {
            return Ok(vec![]);
        };
        if !root.try_exists().unwrap_or(true) {
            return Ok(vec![]);
        }
        let readable = fs::symlink_metadata(&root)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            && fs::read_dir(&root).is_ok();
        Ok(vec![AgentInstallation {
            id: "antigravity:default".into(),
            provider: self.id(),
            platform: host_platform(),
            version: None,
            data_roots: vec![root.display().to_string()],
            status: if readable {
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
        let mut inspection = ProviderInspection {
            version: None,
            schema_versions: BTreeMap::new(),
            journal_modes: BTreeMap::new(),
            agent_running: Self::running(),
            readable_roots: installation
                .data_roots
                .iter()
                .map(|p| {
                    (
                        p.clone(),
                        fs::symlink_metadata(p)
                            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
                            && fs::read_dir(p).is_ok(),
                    )
                })
                .collect(),
            unknown_structures: vec![
                "Brain artifacts and fork/subagent dependencies remain report-only".into(),
            ],
            problems: BTreeMap::new(),
        };
        match Self::root(installation).and_then(Self::open) {
            Ok(db) => {
                inspection.schema_versions.insert(
                    "conversation_summaries".into(),
                    "observed-projection-v1".into(),
                );
                if let Ok(mode) = db.journal_mode() {
                    inspection
                        .journal_modes
                        .insert("conversation_summaries".into(), mode);
                }
            }
            Err(error) => {
                inspection.problems.insert(
                    "antigravity-index-unavailable".into(),
                    ScanProblem::Warning {
                        message: format!("Antigravity index unavailable: {error}"),
                    },
                );
            }
        }
        Ok(inspection)
    }
    async fn capabilities(
        &self,
        inspection: &ProviderInspection,
    ) -> anyhow::Result<AgentCapabilities> {
        let status = if inspection.readable_roots.values().any(|r| !r) {
            CapabilityStatus::PermissionRequired
        } else if !inspection.problems.is_empty() {
            CapabilityStatus::Degraded
        } else {
            CapabilityStatus::ReadOnly
        };
        Ok(AgentCapabilities::build([
            (CapabilityTopic::Sessions, status),
            (CapabilityTopic::Projects, status),
        ]))
    }
    async fn build_cleanup_units(
        &self,
        snapshot: &AgentSnapshot,
    ) -> anyhow::Result<Vec<CleanupUnit>> {
        cleanup::units(snapshot)
    }
    async fn validate_cleanup_unit(
        &self,
        unit: &CleanupUnit,
        installation: &AgentInstallation,
    ) -> anyhow::Result<Vec<CleanupPrecondition>> {
        cleanup::validate(unit, installation)
    }
    async fn execute_provider_operation(
        &self,
        item: &CleanupItem,
        installation: &AgentInstallation,
    ) -> anyhow::Result<CleanupItemOutcome> {
        cleanup::execute(item, installation)
    }
    async fn scan(
        &self,
        installation: &AgentInstallation,
        options: &ScanOptions,
    ) -> anyhow::Result<AgentSnapshot> {
        Self::scan_root(installation, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "00000000-0000-4000-8000-000000000002";
    struct Sample(PathBuf);
    impl Sample {
        fn new() -> Self {
            static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "agenttidy-antigravity-{}-{}-{sequence}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(path.join("conversations")).unwrap();
            fs::create_dir_all(path.join("brain").join(ID)).unwrap();
            let db = rusqlite::Connection::open(path.join(INDEX)).unwrap();
            db.execute_batch(include_str!(
                "../../../fixtures/antigravity/index/schema.sql"
            ))
            .unwrap();
            let row: serde_json::Value =
                serde_json::from_str(include_str!("../../../fixtures/antigravity/index/row.json"))
                    .unwrap();
            db.execute("INSERT INTO conversation_summaries (conversation_id,title,last_modified_time,last_user_input_time,workspace_uris,status) VALUES (?1,?2,?3,?3,?4,?5)",rusqlite::params![row["conversation_id"].as_str().unwrap(),row["title"].as_str().unwrap(),row["last_modified_time"].as_str().unwrap(),row["workspace_uris"].as_str().unwrap(),row["status"].as_str().unwrap()]).unwrap();
            // Opaque files simulate filesystem presence only; their contents are
            // not asserted as a supported protobuf format or added as fixtures.
            fs::write(
                path.join("conversations").join(format!("{ID}.pb")),
                b"opaque",
            )
            .unwrap();
            fs::write(
                path.join("brain").join(ID).join("artifact.md"),
                b"unread body",
            )
            .unwrap();
            Self(path)
        }
        fn install(&self) -> AgentInstallation {
            AgentInstallation {
                id: "antigravity:default".into(),
                provider: ProviderId::new(ProviderId::ANTIGRAVITY),
                platform: Platform::Macos,
                version: None,
                data_roots: vec![self.0.display().to_string()],
                status: InstallationStatus::Available,
            }
        }
        fn scan(&self) -> AgentSnapshot {
            AntigravityAdapter::scan_root(&self.install(), &ScanOptions::default()).unwrap()
        }
    }
    impl Drop for Sample {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }
    #[tokio::test]
    async fn real_index_projection_links_files_without_enabling_mutation() {
        let sample = Sample::new();
        let snapshot = sample.scan();
        assert!(snapshot.problems.is_empty());
        assert_eq!(snapshot.sessions.len(), 1);
        let session = &snapshot.sessions[0];
        assert_eq!(session.title.as_deref(), Some("Sanitized session"));
        assert_eq!(session.lifecycle, SessionLifecycle::Unknown);
        assert_eq!(
            session.updated_at,
            epoch("2026-02-03 01:14:51.153367+00:00")
        );
        assert!(
            session.project.is_none(),
            "multi-root workspace must remain unassigned"
        );
        assert_eq!(session.size.logical_bytes, 17);
        assert_eq!(session.resource_refs.len(), 2);
        assert!(snapshot
            .resources
            .iter()
            .all(|r| r.size.reclaimable_bytes.is_none() && r.ownership != Ownership::Exclusive));
        assert!(AntigravityAdapter
            .build_cleanup_units(&snapshot)
            .await
            .unwrap()
            .is_empty());
        let caps = AntigravityAdapter
            .capabilities(&AntigravityAdapter.inspect(&sample.install()).await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            caps.get(CapabilityTopic::Sessions),
            CapabilityStatus::ReadOnly
        );
        assert_eq!(
            caps.get(CapabilityTopic::Cleanup),
            CapabilityStatus::Unsupported
        );
        assert_eq!(
            caps.get(CapabilityTopic::Archive),
            CapabilityStatus::Unsupported
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn single_workspace_uri_is_decoded_without_traversing_the_project() {
        let sample = Sample::new();
        let db = rusqlite::Connection::open(sample.0.join(INDEX)).unwrap();
        db.execute(
            "UPDATE conversation_summaries SET workspace_uris=?1",
            [r#"["file:///Users/sample/My%20Project"]"#],
        )
        .unwrap();
        drop(db);
        assert_eq!(
            sample.scan().sessions[0]
                .project
                .as_ref()
                .unwrap()
                .cwd
                .as_deref(),
            Some("/Users/sample/My Project")
        );
    }

    /// A Windows host stores the workspace URI with a drive letter
    /// (`file:///c%3A/...`), and every row of the observed Windows index
    /// carries an **empty** title. The URI must decode to a usable
    /// Windows path and the empty title must degrade to "no title" —
    /// never a blank title the user would read as a rendering bug.
    #[cfg(windows)]
    #[test]
    fn windows_workspace_uri_decodes_and_an_empty_title_degrades() {
        let sample = Sample::new();
        let db = rusqlite::Connection::open(sample.0.join(INDEX)).unwrap();
        db.execute(
            "UPDATE conversation_summaries SET workspace_uris=?1, title=''",
            [r#"["file:///c%3A/Sample/project"]"#],
        )
        .unwrap();
        drop(db);
        let session = &sample.scan().sessions[0];
        assert_eq!(
            session.project.as_ref().unwrap().cwd.as_deref(),
            Some(r"c:\Sample\project"),
            "a Windows drive-letter URI must decode to a Windows path"
        );
        assert_eq!(
            session.title, None,
            "an empty index title must degrade to no title"
        );
    }

    /// The empty-title rule holds on every platform: a row whose title
    /// is the empty string must not surface as a blank title.
    #[test]
    fn an_empty_index_title_degrades_to_no_title() {
        let sample = Sample::new();
        let db = rusqlite::Connection::open(sample.0.join(INDEX)).unwrap();
        db.execute("UPDATE conversation_summaries SET title=''", [])
            .unwrap();
        drop(db);
        assert_eq!(sample.scan().sessions[0].title, None);
    }
    #[test]
    fn missing_index_and_changed_schema_degrade_instead_of_guessing_sessions() {
        let sample = Sample::new();
        let db = rusqlite::Connection::open(sample.0.join(INDEX)).unwrap();
        db.execute(
            "ALTER TABLE conversation_summaries RENAME COLUMN status TO future_status",
            [],
        )
        .unwrap();
        drop(db);
        let snapshot = sample.scan();
        assert!(snapshot.sessions.is_empty());
        assert!(snapshot
            .problems
            .contains_key("antigravity-index-unavailable"));
        fs::remove_file(sample.0.join(INDEX)).unwrap();
        assert!(sample.scan().sessions.is_empty());
    }
    #[test]
    fn missing_transcript_and_unindexed_files_are_reported() {
        let sample = Sample::new();
        fs::remove_file(sample.0.join("conversations").join(format!("{ID}.pb"))).unwrap();
        fs::write(sample.0.join("conversations/orphan.pb"), b"orphan").unwrap();
        let snapshot = sample.scan();
        assert!(snapshot
            .problems
            .contains_key("antigravity-unindexed-files"));
        assert!(snapshot
            .problems
            .keys()
            .any(|k| k.starts_with("antigravity-transcript-missing:")));
        assert!(!snapshot
            .resources
            .iter()
            .any(|r| r.kind == ResourceKind::Unknown));
        let snapshot = AntigravityAdapter::scan_root(
            &sample.install(),
            &ScanOptions {
                include_unknown: true,
            },
        )
        .unwrap();
        assert!(snapshot
            .resources
            .iter()
            .any(|r| r.kind == ResourceKind::Unknown));
    }
    #[test]
    fn malformed_ids_and_pre_epoch_timestamps_stay_unknown() {
        assert!(!valid_id("../../outside"));
        assert!(!valid_id("bad-id"));
        assert!(valid_id(ID));
        assert_eq!(epoch("0001-01-01 00:00:00+00:00"), None);
        assert_eq!(epoch("invalid"), None);
        let sample = Sample::new();
        let db = rusqlite::Connection::open(sample.0.join(INDEX)).unwrap();
        db.execute(
            "UPDATE conversation_summaries SET conversation_id='../escape'",
            [],
        )
        .unwrap();
        drop(db);
        assert!(sample.scan().sessions.is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn linked_index_and_artifacts_are_not_followed_and_hardlinks_count_once() {
        let sample = Sample::new();
        let outside = Sample::new();
        std::os::unix::fs::symlink(
            outside.0.join("conversations").join(format!("{ID}.pb")),
            sample.0.join("brain").join(ID).join("link"),
        )
        .unwrap();
        fs::hard_link(
            sample.0.join("conversations").join(format!("{ID}.pb")),
            sample.0.join("brain").join(ID).join("hardlink"),
        )
        .unwrap();
        assert_eq!(sample.scan().sessions[0].size.logical_bytes, 17);
        fs::remove_file(sample.0.join(INDEX)).unwrap();
        std::os::unix::fs::symlink(outside.0.join(INDEX), sample.0.join(INDEX)).unwrap();
        assert!(sample.scan().sessions.is_empty());
    }
    #[test]
    fn reads_live_wal_metadata_without_modifying_index() {
        let sample = Sample::new();
        let db = rusqlite::Connection::open(sample.0.join(INDEX)).unwrap();
        db.pragma_update(None, "journal_mode", "wal").unwrap();
        db.execute("UPDATE conversation_summaries SET title='WAL title'", [])
            .unwrap();
        assert_eq!(
            sample.scan().sessions[0].title.as_deref(),
            Some("WAL title")
        );
        assert_eq!(
            db.query_row("SELECT title FROM conversation_summaries", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "WAL title"
        );
    }
}
