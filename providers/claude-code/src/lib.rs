//! Read-only Claude Code CLI provider adapter (`Start.md` §10, Phase 3).
//!
//! This adapter recognizes only the verified CLI transcript layout under
//! `~/.claude/projects/`. It reads compact line metadata while deliberately
//! ignoring message bodies, and treats unknown structures as diagnostics. The
//! separate Windows Claude Desktop VM installation is discovered later; its
//! credentials and VM images are outside this adapter's scan scope.

use agenttidy_core::{
    AgentCapabilities, AgentInstallation, AgentSnapshot, CapabilityStatus, CapabilityTopic,
    CleanupAction, CleanupLocator, CleanupOperation, CleanupPrecondition, CleanupPreconditionKind,
    CleanupUnit, CleanupUnitKind, FileIdentity, InstallationStatus, ManagedBy, Ownership, Platform,
    ProjectRef, ProviderId, Resource, ResourceFingerprint, ResourceId, ResourceKind,
    ResourceLocator, ResourceRef, ScanOptions, ScanProblem, Session, SessionId, SessionLifecycle,
    SizeConfidence, SizeInfo,
};
use agenttidy_infrastructure::disk_usage::UsageAccumulator;
use agenttidy_infrastructure::fs_probe::{file_identity, walk_tree, EntryKind, WalkProblem};
use agenttidy_infrastructure::paths::{has_reserved_component, is_within, safe_canonicalize};
use agenttidy_infrastructure::processes::{any_process_running, ProcessSignature};
use agenttidy_provider_api::{AgentProviderAdapter, ProviderInspection};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const INSTALLATION_ID: &str = "claude-code:cli";

/// Read-only adapter for the standalone Claude Code CLI installation.
#[derive(Debug, Default)]
pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
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

    fn root(home: &Path) -> PathBuf {
        home.join(".claude")
    }

    fn running() -> bool {
        // Presence is only a conservative active-data signal (§16.2), never
        // permission to mutate a transcript.
        any_process_running(&[ProcessSignature::new(&["claude", "claude-code"], &[])])
    }

    fn is_readable_dir(path: &Path) -> bool {
        fs::read_dir(path).is_ok()
    }

    fn scan_projects(
        root: &Path,
        installation: &AgentInstallation,
        active: bool,
        options: &ScanOptions,
    ) -> (Vec<Session>, Vec<Resource>, BTreeMap<String, ScanProblem>) {
        let mut sessions = Vec::new();
        let mut resources = Vec::new();
        let mut problems = BTreeMap::new();
        let mut seen_paths = HashSet::new();
        let projects = root.join("projects");
        if !projects.is_dir() {
            problems.insert(
                "projects-missing".into(),
                ScanProblem::Warning {
                    message: "projects directory missing; no CLI sessions found".into(),
                },
            );
            return (sessions, resources, problems);
        }

        let project_dirs = match fs::read_dir(&projects) {
            Ok(entries) => entries,
            Err(error) => {
                problems.insert(
                    "projects-unreadable".into(),
                    ScanProblem::Error {
                        message: error.to_string(),
                        path: Some(projects.display().to_string()),
                    },
                );
                return (sessions, resources, problems);
            }
        };
        for project_dir in project_dirs.flatten() {
            let path = project_dir.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                problems.insert(
                    format!("project-unreadable:{}", path.display()),
                    ScanProblem::Error {
                        message: "cannot stat project directory".into(),
                        path: Some(path.display().to_string()),
                    },
                );
                continue;
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let outcome = walk_tree(&path);
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
                let is_direct_transcript = entry.kind == EntryKind::File
                    && entry.path.extension().and_then(|part| part.to_str()) == Some("jsonl")
                    && entry
                        .path
                        .strip_prefix(&path)
                        .ok()
                        .is_some_and(|relative| relative.components().count() == 1);
                if !is_direct_transcript || !seen_paths.insert(entry.path.clone()) {
                    continue;
                }
                Self::record_session(
                    &entry.path,
                    &path,
                    installation,
                    active,
                    options,
                    &mut sessions,
                    &mut resources,
                    &mut problems,
                );
            }
        }
        (sessions, resources, problems)
    }

    /// Desktop mirrors CLI transcripts below per-session `.claude` roots.
    /// Only those verified transcript trees are scanned; the surrounding VM,
    /// credentials, audit logs and user outputs remain entirely out of scope.
    fn scan_desktop_projects(
        root: &Path,
        installation: &AgentInstallation,
        options: &ScanOptions,
    ) -> (Vec<Session>, Vec<Resource>, BTreeMap<String, ScanProblem>) {
        let mut sessions = Vec::new();
        let mut resources = Vec::new();
        let mut problems = BTreeMap::new();
        let sessions_root = root.join("local-agent-mode-sessions");
        if !sessions_root.is_dir() {
            problems.insert(
                "desktop-sessions-missing".into(),
                ScanProblem::Warning {
                    message: "Claude Desktop has no local-agent-mode sessions".into(),
                },
            );
            return (sessions, resources, problems);
        }
        for entry in walk_tree(&sessions_root).entries {
            if entry.kind != EntryKind::Dir
                || entry.path.file_name().and_then(|name| name.to_str()) != Some(".claude")
            {
                continue;
            }
            let (found_sessions, found_resources, found_problems) =
                Self::scan_projects(&entry.path, installation, Self::running(), options);
            sessions.extend(found_sessions);
            resources.extend(found_resources);
            problems.extend(found_problems);
        }
        (sessions, resources, problems)
    }

    #[allow(clippy::too_many_arguments)]
    fn record_session(
        transcript: &Path,
        project_dir: &Path,
        installation: &AgentInstallation,
        active: bool,
        options: &ScanOptions,
        sessions: &mut Vec<Session>,
        resources: &mut Vec<Resource>,
        problems: &mut BTreeMap<String, ScanProblem>,
    ) {
        let meta = match Self::first_line_metadata(transcript) {
            Ok(meta) => meta,
            Err(error) => {
                Self::record_unknown(
                    transcript,
                    installation,
                    options,
                    resources,
                    problems,
                    error,
                );
                return;
            }
        };
        let Some(id) = meta.session_id else {
            Self::record_unknown(
                transcript,
                installation,
                options,
                resources,
                problems,
                "transcript metadata has no sessionId".into(),
            );
            return;
        };
        let file_id = transcript.file_stem().and_then(|value| value.to_str());
        if file_id != Some(id.as_str()) {
            Self::record_unknown(
                transcript,
                installation,
                options,
                resources,
                problems,
                "sessionId does not match transcript filename".into(),
            );
            return;
        }

        let side_dir = project_dir.join(&id);
        let mut paths = vec![transcript.to_path_buf()];
        if side_dir.is_dir() {
            paths.push(side_dir);
        }
        let (size, measurement_problems) = Self::measure_unit(&paths);
        for (key, problem) in measurement_problems {
            problems.insert(key, problem);
        }
        let session_id = SessionId::new(id.clone());
        let resource_id = ResourceId::new(format!(
            "{INSTALLATION_ID}:session:{}:{}",
            project_dir.display(),
            id
        ));
        let project = meta.cwd.map(ProjectRef::from_cwd);
        let lifecycle = if active {
            // A global Claude process cannot identify the writer's session;
            // Unknown prevents later policy from treating it as inactive.
            SessionLifecycle::Unknown
        } else {
            SessionLifecycle::Inactive
        };
        let mut metadata = serde_json::Map::new();
        if let Some(version) = meta.version {
            metadata.insert("version".into(), serde_json::Value::String(version));
        }
        resources.push(Resource {
            id: resource_id.clone(),
            provider: ProviderId::new(ProviderId::CLAUDE_CODE),
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
            provider: ProviderId::new(ProviderId::CLAUDE_CODE),
            installation_id: installation.id.clone(),
            title: None,
            project,
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

    fn first_line_metadata(path: &Path) -> Result<ClaudeLine, String> {
        let file = fs::File::open(path).map_err(|error| error.to_string())?;
        let line = BufReader::new(file)
            .lines()
            .next()
            .ok_or_else(|| "empty transcript".to_string())?
            .map_err(|error| error.to_string())?;
        // `ClaudeLine` deliberately declares no message/content field: serde
        // skips those bodies while extracting only schema metadata.
        serde_json::from_str(&line).map_err(|error| error.to_string())
    }

    fn measure_unit(paths: &[PathBuf]) -> (SizeInfo, BTreeMap<String, ScanProblem>) {
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
        let confidence = if usage.unidentifiable_files == 0 {
            SizeConfidence::Exact
        } else {
            SizeConfidence::Estimated
        };
        (
            SizeInfo {
                logical_bytes: usage.logical_bytes,
                allocated_bytes: usage.allocated_bytes,
                exclusive_bytes: Some(usage.logical_bytes),
                confidence,
                ..SizeInfo::default()
            },
            problems,
        )
    }

    fn record_unknown(
        transcript: &Path,
        installation: &AgentInstallation,
        options: &ScanOptions,
        resources: &mut Vec<Resource>,
        problems: &mut BTreeMap<String, ScanProblem>,
        reason: String,
    ) {
        let path = transcript.display().to_string();
        problems.insert(
            format!("unrecognized-transcript:{path}"),
            ScanProblem::Warning { message: reason },
        );
        if options.include_unknown {
            resources.push(Resource {
                id: ResourceId::new(format!("{INSTALLATION_ID}:unknown:{path}")),
                provider: ProviderId::new(ProviderId::CLAUDE_CODE),
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
struct ClaudeLine {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    version: Option<String>,
}

#[async_trait::async_trait]
impl AgentProviderAdapter for ClaudeCodeAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::new(ProviderId::CLAUDE_CODE)
    }

    fn display_name(&self) -> &str {
        "Claude Code"
    }

    async fn detect(&self) -> anyhow::Result<Vec<AgentInstallation>> {
        let Some(home) = Self::home_dir() else {
            return Ok(vec![]);
        };
        let root = Self::root(&home);
        let mut installations = Vec::new();
        if root.exists() {
            installations.push(AgentInstallation {
                id: INSTALLATION_ID.into(),
                provider: self.id(),
                platform: Self::platform(),
                version: None,
                data_roots: vec![root.display().to_string()],
                status: if Self::is_readable_dir(&root) {
                    InstallationStatus::Available
                } else {
                    InstallationStatus::PermissionRequired
                },
            });
        }
        #[cfg(windows)]
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            let desktop_root = PathBuf::from(local_app_data).join("Claude-3p");
            if desktop_root.exists() {
                installations.push(AgentInstallation {
                    id: "claude-code:desktop".into(),
                    provider: self.id(),
                    platform: Platform::Windows,
                    version: None,
                    data_roots: vec![desktop_root.display().to_string()],
                    status: if Self::is_readable_dir(&desktop_root) {
                        InstallationStatus::Available
                    } else {
                        InstallationStatus::PermissionRequired
                    },
                });
            }
        }
        Ok(installations)
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
        Ok(ProviderInspection {
            version: installation.version.clone(),
            schema_versions: BTreeMap::new(),
            journal_modes: BTreeMap::new(),
            agent_running: Self::running(),
            readable_roots,
            unknown_structures: vec![],
            problems: BTreeMap::new(),
        })
    }

    async fn capabilities(
        &self,
        inspection: &ProviderInspection,
    ) -> anyhow::Result<AgentCapabilities> {
        let readable = !inspection.readable_roots.is_empty()
            && inspection.readable_roots.values().all(|readable| *readable);
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
            .ok_or_else(|| anyhow::anyhow!("Claude Code installation has no state root"))?;
        let (sessions, resources, problems) = if installation.id == "claude-code:desktop" {
            Self::scan_desktop_projects(&root, installation, options)
        } else {
            Self::scan_projects(&root, installation, Self::running(), options)
        };
        Ok(AgentSnapshot {
            installation: installation.clone(),
            sessions,
            resources,
            problems,
            completed_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64,
        })
    }

    /// Phase 7 beta: transcript `FileSet` cleanup units
    /// (`docs/safety/session-transcript-fileset.md`).
    ///
    /// One unit per recognized, exclusively-owned session resource: the
    /// transcript `*.jsonl` plus its same-named side directory — exactly
    /// the file set the scan already measured (§9.5 atomicity). Structural
    /// facts only: which sessions *should* be cleaned is the policy's
    /// decision (§10). Unrecognized / mismatched transcripts stay
    /// report-only and never form units (§3.2 + §7.3).
    async fn build_cleanup_units(
        &self,
        snapshot: &AgentSnapshot,
    ) -> anyhow::Result<Vec<CleanupUnit>> {
        let mut units = Vec::new();
        for resource in &snapshot.resources {
            if resource.kind != ResourceKind::Session || resource.ownership != Ownership::Exclusive
            {
                continue;
            }
            let Some(session_id) = &resource.session_id else {
                continue;
            };
            let ResourceLocator::FileSet { paths } = &resource.locator else {
                continue;
            };
            let Some(primary) = paths.first().map(PathBuf::from) else {
                continue;
            };
            // The transcript (primary path) carries the §12.2
            // fingerprint: identity + size + mtime. A path that cannot be
            // fingerprinted cannot be safely revalidated later, so its
            // resource stays report-only (fail-closed).
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
                provider: ProviderId::new(ProviderId::CLAUDE_CODE),
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

    /// §16.1 path safety for a transcript file set: every member path
    /// must canonicalize inside the installation's data roots (no symlink
    /// escapes), keep the verified `projects/<slug>/` layout shared by CLI
    /// transcripts and Desktop mirrors, and share one project directory
    /// (transcript + side directory). Rejects empty / reserved / over-broad
    /// paths. Returns the typed §12.2 preconditions the revalidator must
    /// check before executing.
    async fn validate_cleanup_unit(
        &self,
        unit: &CleanupUnit,
        installation: &AgentInstallation,
    ) -> anyhow::Result<Vec<CleanupPrecondition>> {
        let CleanupLocator::FileSet { paths } = &unit.locator else {
            anyhow::bail!("claude-code cleanup units must be FileSet locators");
        };
        let Some(primary) = paths.first() else {
            anyhow::bail!("empty file set");
        };
        let Some(shared_dir) = primary.parent() else {
            anyhow::bail!(
                "file set path has no parent directory: {}",
                primary.display()
            );
        };
        let roots = installation
            .data_roots
            .iter()
            .map(|root| safe_canonicalize(Path::new(root)))
            .collect::<Result<Vec<_>, _>>()?;
        for path in paths {
            if path.as_os_str().is_empty() {
                anyhow::bail!("empty path in file set");
            }
            if has_reserved_component(path) {
                anyhow::bail!("reserved path component in file set: {}", path.display());
            }
            if path.parent() != Some(shared_dir) {
                anyhow::bail!(
                    "file set members must share one project directory (transcript + side directory): {}",
                    path.display()
                );
            }
            // Canonicalize first: a member that is a symlink pointing out
            // of the data root lands outside `roots` and is rejected
            // (§16.1 "不跟随指向数据根目录之外的 Symlink").
            let canonical = safe_canonicalize(path).map_err(|error| {
                anyhow::anyhow!("cannot canonicalize {}: {error}", path.display())
            })?;
            if !roots.iter().any(|root| is_within(root, &canonical)) {
                anyhow::bail!("unit path escapes provider data roots: {}", path.display());
            }
        }
        // Layout invariant: transcripts live at `<root>/projects/<slug>/`
        // for both the CLI installation and the Desktop mirrors — a shared
        // dir whose parent is not `projects` is not a verified transcript
        // location and must not be cleaned.
        let inside_projects = shared_dir.parent().and_then(|parent| parent.file_name())
            == Some(std::ffi::OsStr::new("projects"));
        if !inside_projects {
            anyhow::bail!(
                "file set is not inside a verified projects/<slug>/ directory: {}",
                shared_dir.display()
            );
        }
        Ok(unit.preconditions.clone())
    }
}

impl ClaudeCodeAdapter {
    /// §12.2 invalidation triggers for a transcript file set
    /// (`docs/safety/session-transcript-fileset.md`). `MtimeUnchanged`
    /// doubles as the session-re-activation trigger: a session that
    /// becomes active writes its transcript, so its mtime moves.
    fn fileset_preconditions() -> Vec<CleanupPrecondition> {
        [
            (
                CleanupPreconditionKind::PathExists,
                "transcript and side directory still exist",
            ),
            (
                CleanupPreconditionKind::PathUnchanged,
                "paths canonicalize to the scanned locations (§16.1)",
            ),
            (
                CleanupPreconditionKind::IdentityUnchanged,
                "transcript file identity unchanged",
            ),
            (
                CleanupPreconditionKind::SizeUnchanged,
                "transcript size unchanged",
            ),
            (
                CleanupPreconditionKind::MtimeUnchanged,
                "transcript mtime unchanged (session-re-activation signal)",
            ),
            (
                CleanupPreconditionKind::NoAgentRunning,
                "no Claude process running (§16.2)",
            ),
            (
                CleanupPreconditionKind::SessionInactive,
                "owning session still Inactive/Archived (§11)",
            ),
        ]
        .into_iter()
        .map(|(kind, description)| CleanupPrecondition {
            kind,
            description: description.into(),
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "agenttidy-claude-code-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("projects/example-project")).unwrap();
        root
    }

    fn installation(root: &Path) -> AgentInstallation {
        AgentInstallation {
            id: INSTALLATION_ID.into(),
            provider: ProviderId::new(ProviderId::CLAUDE_CODE),
            platform: ClaudeCodeAdapter::platform(),
            version: None,
            data_roots: vec![root.display().to_string()],
            status: InstallationStatus::Available,
        }
    }

    #[test]
    fn scan_groups_transcript_and_side_directory_without_parsing_message_content() {
        let root = temp_root();
        let project = root.join("projects/example-project");
        let id = "9521178d-ae3a-4959-b9ac-7591e7eaf5d6";
        fs::write(
            project.join(format!("{id}.jsonl")),
            format!("{{\"type\":\"user\",\"sessionId\":\"{id}\",\"cwd\":\"/tmp/project\",\"version\":\"2.1.218\",\"message\":{{\"content\":\"not retained\"}}}}\n"),
        )
        .unwrap();
        fs::create_dir_all(project.join(id).join("subagents")).unwrap();
        fs::write(
            project.join(id).join("subagents/agent-1.jsonl"),
            "subagent body",
        )
        .unwrap();
        let installation = installation(&root);
        let (sessions, resources, problems) =
            ClaudeCodeAdapter::scan_projects(&root, &installation, false, &ScanOptions::default());

        assert!(problems.is_empty());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_str(), id);
        assert_eq!(
            sessions[0].project.as_ref().unwrap().cwd.as_deref(),
            Some("/tmp/project")
        );
        assert_eq!(resources.len(), 1);
        assert!(matches!(
            resources[0].locator,
            ResourceLocator::FileSet { .. }
        ));
        assert!(resources[0].size.logical_bytes > 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mismatched_filename_is_blocked_unless_unknowns_are_requested() {
        let root = temp_root();
        fs::write(
            root.join("projects/example-project/not-the-session-id.jsonl"),
            "{\"type\":\"user\",\"sessionId\":\"actual-id\"}\n",
        )
        .unwrap();
        let installation = installation(&root);
        let (sessions, resources, problems) =
            ClaudeCodeAdapter::scan_projects(&root, &installation, false, &ScanOptions::default());
        assert!(sessions.is_empty());
        assert!(resources.is_empty());
        assert_eq!(problems.len(), 1);

        let (_, resources, _) = ClaudeCodeAdapter::scan_projects(
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

    // -----------------------------------------------------------------
    // Phase 7 cleanup-unit contract (§9.5 + §16.1 + §12.2).
    // -----------------------------------------------------------------

    /// Snapshot assembled the way `scan` assembles it, from the temp
    /// fixture tree — kept sync so the async tests stay small.
    fn snapshot_of(root: &Path) -> AgentSnapshot {
        let installation = installation(root);
        let (sessions, resources, problems) =
            ClaudeCodeAdapter::scan_projects(root, &installation, false, &ScanOptions::default());
        AgentSnapshot {
            installation,
            sessions,
            resources,
            problems,
            completed_at: 0,
        }
    }

    fn write_session_fixture(root: &Path) -> String {
        let project = root.join("projects/example-project");
        let id = "1d9d0f1e-5a1c-4d0c-9c0b-1a2b3c4d5e6f".to_string();
        fs::write(
            project.join(format!("{id}.jsonl")),
            format!("{{\"sessionId\":\"{id}\"}}\n"),
        )
        .unwrap();
        fs::create_dir_all(project.join(&id).join("subagents")).unwrap();
        id
    }

    #[tokio::test]
    async fn build_cleanup_units_forms_one_fileset_unit_per_session_resource() {
        let root = temp_root();
        let id = write_session_fixture(&root);
        let project = root.join("projects/example-project");
        let snapshot = snapshot_of(&root);

        let units = ClaudeCodeAdapter
            .build_cleanup_units(&snapshot)
            .await
            .expect("units build");
        assert_eq!(units.len(), 1);
        let unit = &units[0];
        // §9.5: one atomic FileSet unit per session, trash-first.
        assert_eq!(unit.kind, CleanupUnitKind::FileSet);
        assert_eq!(unit.operation.action, CleanupAction::Trash);
        assert_eq!(unit.resource_kind, ResourceKind::Session);
        assert_eq!(
            unit.estimated_reclaimable_bytes,
            snapshot.resources[0].size.logical_bytes
        );
        // The whole measured file set travels in the locator: transcript
        // + same-named side directory.
        assert_eq!(unit.locator.paths().len(), 2);
        // The fingerprint is taken from the transcript (primary member).
        assert_eq!(unit.fingerprint.path, project.join(format!("{id}.jsonl")));
        assert!(unit.fingerprint.size_bytes > 0);
        assert!(unit
            .preconditions
            .iter()
            .any(|p| p.kind == CleanupPreconditionKind::NoAgentRunning));
        assert!(unit
            .preconditions
            .iter()
            .any(|p| p.kind == CleanupPreconditionKind::MtimeUnchanged));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn unrecognized_transcripts_never_form_units() {
        // §3.2 + §7.3: unknown / mismatched transcripts stay report-only.
        let root = temp_root();
        fs::write(
            root.join("projects/example-project/not-the-session-id.jsonl"),
            "{\"sessionId\":\"actual-id\"}\n",
        )
        .unwrap();
        let snapshot = snapshot_of(&root);
        let units = ClaudeCodeAdapter
            .build_cleanup_units(&snapshot)
            .await
            .expect("units build");
        assert!(units.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn validate_accepts_verified_layout_and_declares_preconditions() {
        let root = temp_root();
        write_session_fixture(&root);
        let snapshot = snapshot_of(&root);
        let units = ClaudeCodeAdapter
            .build_cleanup_units(&snapshot)
            .await
            .expect("units build");

        let preconditions = ClaudeCodeAdapter
            .validate_cleanup_unit(&units[0], &snapshot.installation)
            .await
            .expect("verified layout validates");
        assert!(preconditions
            .iter()
            .any(|p| p.kind == CleanupPreconditionKind::PathUnchanged));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn validate_rejects_paths_outside_the_data_root() {
        // §16.1: a unit may never reach outside the provider data root.
        let root = temp_root();
        write_session_fixture(&root);
        let outside = std::env::temp_dir().join(format!(
            "agenttidy-claude-outside-{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&outside, b"x").unwrap();
        let snapshot = snapshot_of(&root);
        let mut units = ClaudeCodeAdapter
            .build_cleanup_units(&snapshot)
            .await
            .expect("units build");
        units[0].locator = CleanupLocator::FileSet {
            paths: vec![outside.clone()],
        };

        let error = ClaudeCodeAdapter
            .validate_cleanup_unit(&units[0], &snapshot.installation)
            .await
            .expect_err("outside-root path must be rejected");
        assert!(error.to_string().contains("escapes provider data roots"));
        fs::remove_file(&outside).ok();
        fs::remove_dir_all(root).unwrap();
    }

    /// §16.1 symlink rule: a member symlinked out of the data root
    /// canonicalizes outside and must be rejected. Unix-only — Windows
    /// symlink creation needs privileges the CI runner may not have.
    #[cfg(unix)]
    #[tokio::test]
    async fn validate_rejects_symlink_escapes() {
        let root = temp_root();
        write_session_fixture(&root);
        let outside = std::env::temp_dir().join(format!(
            "agenttidy-claude-outside-link-{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&outside, b"x").unwrap();
        let link = root.join("projects/example-project/link.jsonl");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let snapshot = snapshot_of(&root);
        let mut units = ClaudeCodeAdapter
            .build_cleanup_units(&snapshot)
            .await
            .expect("units build");
        units[0].locator = CleanupLocator::FileSet { paths: vec![link] };

        let error = ClaudeCodeAdapter
            .validate_cleanup_unit(&units[0], &snapshot.installation)
            .await
            .expect_err("symlink escape must be rejected");
        assert!(error.to_string().contains("escapes provider data roots"));
        fs::remove_file(&outside).ok();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn validate_rejects_layout_outside_projects() {
        // Layout invariant: only `<root>/projects/<slug>/` is a verified
        // transcript location (both CLI and Desktop mirrors).
        let root = temp_root();
        write_session_fixture(&root);
        let stray = root.join("other/stray.jsonl");
        fs::create_dir_all(root.join("other")).unwrap();
        fs::write(&stray, b"x").unwrap();
        let snapshot = snapshot_of(&root);
        let mut units = ClaudeCodeAdapter
            .build_cleanup_units(&snapshot)
            .await
            .expect("units build");
        units[0].locator = CleanupLocator::FileSet { paths: vec![stray] };

        let error = ClaudeCodeAdapter
            .validate_cleanup_unit(&units[0], &snapshot.installation)
            .await
            .expect_err("non-projects layout must be rejected");
        assert!(error.to_string().contains("projects"));
        fs::remove_dir_all(root).unwrap();
    }
}
