//! Read-only WorkBuddy provider adapter (`Start.md` §10, Phase 3).
//!
//! The adapter recognizes only the Phase 0-verified session file set. The
//! WorkBuddy database is opened through the infrastructure read-only guard
//! for inspection facts; until the `sessions` schema has a contract fixture,
//! it is never used to infer lifecycle or cleanup eligibility.

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
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const INSTALLATION_ID: &str = "workbuddy:default";

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
        any_process_running(&[ProcessSignature::new(&["WorkBuddy", "workbuddy"], &[])])
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
        let mut problems = BTreeMap::new();
        if let Some(root) = installation.data_roots.first() {
            let db_path = Path::new(root).join("workbuddy.db");
            match ReadOnlyDb::open(&db_path) {
                Ok(db) => match db.journal_mode() {
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
                },
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
            schema_versions: BTreeMap::new(),
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
        let (sessions, mut resources, problems) = Self::scan_projects(&root, installation, options);
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

    /// Phase 6: no cleanup units. WorkBuddy's `sessions` table contract
    /// is still unfrozen (see `docs/CHANGELOG.md` Phase 3 notes), its
    /// session lifecycle stays `Unknown`, and automations may reference
    /// arbitrary cwds — all of which force Blocked until a Phase 7
    /// safety doc lands for each enabled resource type.
    async fn build_cleanup_units(
        &self,
        snapshot: &AgentSnapshot,
    ) -> anyhow::Result<Vec<CleanupUnit>> {
        let _ = snapshot;
        Ok(Vec::new())
    }

    /// Phase 6: no units, no provider-specific preconditions.
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
    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "agenttidy-workbuddy-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
}
