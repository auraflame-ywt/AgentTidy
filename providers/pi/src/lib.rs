//! Pi v3 metadata-only scanning (`Start.md` §10, §18, §22).
//!
//! Only bounded session headers are parsed. Branch entries, messages, auth and
//! settings remain outside the verified scope. Single-file Trash cleanup is
//! documented in `docs/safety/pi-session-fileset.md`.

use agenttidy_core::*;
use agenttidy_infrastructure::{
    fs_probe::{is_single_link_file, walk_tree, EntryKind},
    paths::{home_dir, host_platform},
    processes::{any_process_running, ProcessSignature},
};
use agenttidy_provider_api::{AgentProviderAdapter, ProviderInspection};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Default)]
pub struct PiAdapter;

mod cleanup;

/// Header limit bounds memory even when a transcript is malformed (§16.1).
const HEADER_LIMIT: u64 = 64 * 1024;

fn millis(time: SystemTime) -> Option<u64> {
    u64::try_from(time.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()
}

fn header(path: &Path) -> anyhow::Result<serde_json::Value> {
    let mut reader = BufReader::new(fs::File::open(path)?.take(HEADER_LIMIT + 1));
    let mut bytes = Vec::new();
    reader.read_until(b'\n', &mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= HEADER_LIMIT,
        "session header exceeds limit"
    );
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        value["type"] == "session" && value["version"] == 3,
        "unsupported Pi session header/version"
    );
    anyhow::ensure!(
        value["id"].as_str().is_some_and(|s| !s.is_empty())
            && value["cwd"].is_string()
            && value["timestamp"].is_string(),
        "incomplete Pi session header"
    );
    Ok(value)
}

impl PiAdapter {
    fn running() -> bool {
        any_process_running(&[ProcessSignature::new(
            &["pi"],
            &["pi-coding-agent", "/bin/pi"],
        )])
    }

    /// Only macOS has real samples; other platform paths await investigation.
    fn discover() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let home = home_dir()?;
            let configured = std::env::var_os("PI_CODING_AGENT_DIR").filter(|s| !s.is_empty());
            let path = configured
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".pi/agent"));
            let path = if let Ok(tail) = path.strip_prefix("~") {
                home.join(tail)
            } else {
                path
            };
            Some(path)
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    fn scan_root(installation: &AgentInstallation) -> anyhow::Result<AgentSnapshot> {
        let root = Path::new(
            installation
                .data_roots
                .first()
                .ok_or_else(|| anyhow::anyhow!("missing Pi root"))?,
        );
        let directory = installation
            .data_roots
            .get(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("sessions"));
        let running = Self::running();
        let mut snapshot = AgentSnapshot {
            installation: installation.clone(),
            sessions: vec![],
            resources: vec![],
            problems: BTreeMap::new(),
            completed_at: millis(SystemTime::now()).unwrap_or(0),
        };
        // Do not follow a redirected session directory into unrelated user data.
        if !fs::symlink_metadata(root).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            || !fs::symlink_metadata(&directory)
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            snapshot.problems.insert(
                "pi-sessions-unavailable".into(),
                ScanProblem::Error {
                    message: "Pi sessions directory missing, unreadable or linked".into(),
                    path: Some(directory.display().to_string()),
                },
            );
            return Ok(snapshot);
        }
        let walk = walk_tree(&directory);
        for problem in walk.problems {
            snapshot.problems.insert(
                format!("pi-walk:{problem:?}"),
                ScanProblem::Warning {
                    message: format!("Incomplete Pi session enumeration: {problem:?}"),
                },
            );
        }
        let mut seen = HashSet::new();
        let mut file_identities = HashSet::new();
        for entry in walk.entries {
            if entry.kind != EntryKind::File
                || entry.path.extension().and_then(|x| x.to_str()) != Some("jsonl")
            {
                continue;
            }
            let meta = match header(&entry.path) {
                Ok(meta) => meta,
                Err(error) => {
                    snapshot.problems.insert(
                        format!("pi-header:{}", entry.path.display()),
                        ScanProblem::Error {
                            message: format!("Pi header unavailable: {error}"),
                            path: Some(entry.path.display().to_string()),
                        },
                    );
                    continue;
                }
            };
            let id = meta["id"].as_str().expect("validated header").to_owned();
            if !seen.insert(id.clone()) {
                snapshot.problems.insert(
                    format!("pi-duplicate:{id}"),
                    ScanProblem::Warning {
                        message: "Duplicate Pi session id; duplicate left unattributed".into(),
                    },
                );
                continue;
            }
            // Hard-linked transcripts occupy bytes once across the snapshot (§8).
            if let Some(identity) = entry.identity {
                if !file_identities.insert(identity) {
                    continue;
                }
            }
            let session_id = SessionId::new(id.clone());
            let resource_id = ResourceId::new(format!("{}:session:{id}", installation.id));
            let project = Some(ProjectRef::from_cwd(
                meta["cwd"].as_str().expect("validated cwd"),
            ));
            let updated_at = fs::metadata(&entry.path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(millis);
            let created_at = meta["timestamp"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .and_then(|d| u64::try_from(d.timestamp_millis()).ok());
            let size = SizeInfo {
                logical_bytes: entry.logical_len,
                allocated_bytes: entry.allocated_len,
                confidence: SizeConfidence::Exact,
                ..SizeInfo::default()
            };
            let mut metadata = serde_json::Map::new();
            metadata.insert("session_format".into(), 3.into());
            // Fork ancestry is a fact, not evidence of exclusive ownership.
            if let Some(parent) = meta.get("parentSession").filter(|v| v.is_string()) {
                metadata.insert("parent_session".into(), parent.clone());
            }
            snapshot.resources.push(Resource {
                id: resource_id.clone(),
                provider: installation.provider.clone(),
                installation_id: installation.id.clone(),
                kind: ResourceKind::Session,
                locator: ResourceLocator::File {
                    path: entry.path.display().to_string(),
                },
                ownership: if is_single_link_file(&entry.path)
                    && cleanup::native_filename(&entry.path, &id)
                {
                    Ownership::Exclusive
                } else {
                    Ownership::Unknown
                },
                managed_by: ManagedBy::Agent,
                size,
                session_id: Some(session_id.clone()),
                project: project.clone(),
                created_at,
                updated_at,
                dependencies: vec![],
                metadata: serde_json::Map::new(),
            });
            snapshot.sessions.push(Session {
                id: session_id,
                provider: installation.provider.clone(),
                installation_id: installation.id.clone(),
                title: None,
                project,
                created_at,
                updated_at,
                lifecycle: if running {
                    SessionLifecycle::Unknown
                } else {
                    SessionLifecycle::Inactive
                },
                size,
                resource_refs: vec![ResourceRef {
                    id: resource_id,
                    kind: ResourceKind::Session,
                }],
                metadata,
            });
        }
        // Duplicate identities cannot prove a unique session resource; downgrade
        // the first member too, rather than merely discarding later aliases.
        for resource in &mut snapshot.resources {
            if resource.session_id.as_ref().is_some_and(|id| {
                snapshot
                    .problems
                    .contains_key(&format!("pi-duplicate:{}", id.0))
            }) {
                resource.ownership = Ownership::Unknown;
            }
        }
        snapshot.sessions.sort_by(|a, b| a.id.0.cmp(&b.id.0));
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
impl AgentProviderAdapter for PiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::new(ProviderId::PI)
    }
    fn display_name(&self) -> &str {
        "Pi"
    }
    async fn detect(&self) -> anyhow::Result<Vec<AgentInstallation>> {
        let Some(root) = Self::discover() else {
            return Ok(vec![]);
        };
        let mut data_roots = vec![root.display().to_string()];
        if let Some(path) = std::env::var_os("PI_CODING_AGENT_SESSION_DIR")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
        {
            let path = if let (Some(home), Ok(tail)) = (home_dir(), path.strip_prefix("~")) {
                home.join(tail)
            } else {
                path
            };
            data_roots.push(path.display().to_string());
        }
        if !root.try_exists().unwrap_or(true)
            && !data_roots.iter().skip(1).any(|p| Path::new(p).exists())
        {
            return Ok(vec![]);
        }
        let readable = fs::symlink_metadata(&root)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            && fs::read_dir(&root).is_ok();
        Ok(vec![AgentInstallation {
            id: "pi:default".into(),
            provider: self.id(),
            platform: host_platform(),
            version: None,
            data_roots,
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
        let snapshot = Self::scan_root(installation)?;
        Ok(ProviderInspection {
            version: None,
            schema_versions: if snapshot.sessions.is_empty() {
                BTreeMap::new()
            } else {
                BTreeMap::from([("session-header".into(), "3".into())])
            },
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
            unknown_structures: vec![],
            problems: snapshot.problems,
        })
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
            (
                CapabilityTopic::Cleanup,
                if status == CapabilityStatus::ReadOnly && !inspection.agent_running {
                    CapabilityStatus::Supported
                } else {
                    status
                },
            ),
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
    async fn scan(
        &self,
        installation: &AgentInstallation,
        _options: &ScanOptions,
    ) -> anyhow::Result<AgentSnapshot> {
        Self::scan_root(installation)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) struct Sample(pub(crate) PathBuf);
    impl Sample {
        pub(crate) fn new() -> Self {
            static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "agenttidy-pi-{}-{}-{sequence}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(p.join("sessions/project")).unwrap();
            Self(p)
        }
        pub(crate) fn install(&self) -> AgentInstallation {
            AgentInstallation {
                id: "pi:default".into(),
                provider: ProviderId::new(ProviderId::PI),
                platform: Platform::Macos,
                version: None,
                data_roots: vec![self.0.display().to_string()],
                status: InstallationStatus::Available,
            }
        }
        pub(crate) fn write(&self, text: &str) -> PathBuf {
            let p = self.0.join("sessions/project/session.jsonl");
            fs::write(&p, text).unwrap();
            p
        }
    }
    impl Drop for Sample {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }
    pub(crate) const HEADER: &str = include_str!("../../../fixtures/pi/sessions/header.jsonl");

    #[tokio::test]
    async fn real_header_maps_metadata_without_reading_body_or_enabling_cleanup() {
        let sample = Sample::new();
        let text = format!("{HEADER}not JSON: transcript body must never be parsed\n");
        let path = sample.write(&text);
        let install = sample.install();
        let snapshot = PiAdapter
            .scan(&install, &ScanOptions::default())
            .await
            .unwrap();
        assert_eq!(snapshot.sessions.len(), 1);
        let session = &snapshot.sessions[0];
        assert!(matches!(
            session.lifecycle,
            SessionLifecycle::Unknown | SessionLifecycle::Inactive
        ));
        assert_eq!(
            session.project.as_ref().unwrap().cwd.as_deref(),
            Some("/Users/sample/project")
        );
        assert!(session.created_at.is_some());
        assert!(session.title.is_none());
        assert_eq!(session.size.logical_bytes, text.len() as u64);
        assert_eq!(snapshot.resources[0].ownership, Ownership::Unknown);
        assert!(PiAdapter
            .build_cleanup_units(&snapshot)
            .await
            .unwrap()
            .is_empty());
        let inspection = PiAdapter.inspect(&install).await.unwrap();
        let caps = PiAdapter.capabilities(&inspection).await.unwrap();
        assert_eq!(
            caps.get(CapabilityTopic::Sessions),
            CapabilityStatus::ReadOnly
        );
        assert!(matches!(
            caps.get(CapabilityTopic::Cleanup),
            CapabilityStatus::Supported | CapabilityStatus::ReadOnly
        ));
        assert_eq!(
            caps.get(CapabilityTopic::Archive),
            CapabilityStatus::Unsupported
        );
        assert_eq!(fs::read_to_string(path).unwrap(), text);
    }
    #[test]
    fn oversized_and_future_headers_fail_closed() {
        let sample = Sample::new();
        let path = sample.write(&"x".repeat(HEADER_LIMIT as usize + 1));
        assert!(header(&path).is_err());
        sample.write(&HEADER.replace("\"version\": 3", "\"version\": 4"));
        let snapshot = PiAdapter::scan_root(&sample.install()).unwrap();
        assert!(snapshot.sessions.is_empty());
        assert!(!snapshot.problems.is_empty());
    }
    #[test]
    fn duplicate_ids_are_reported() {
        let sample = Sample::new();
        sample.write(HEADER);
        fs::write(sample.0.join("sessions/project/copy.jsonl"), HEADER).unwrap();
        let snapshot = PiAdapter::scan_root(&sample.install()).unwrap();
        assert_eq!(snapshot.sessions.len(), 1);
        assert!(snapshot
            .problems
            .keys()
            .any(|k| k.starts_with("pi-duplicate:")));
        assert_eq!(snapshot.total_size().confidence, SizeConfidence::Estimated);
    }
    #[test]
    fn separate_session_root_is_used_without_an_extra_sessions_component() {
        let sample = Sample::new();
        sample.write(HEADER);
        let mut install = sample.install();
        install
            .data_roots
            .push(sample.0.join("sessions/project").display().to_string());
        assert_eq!(PiAdapter::scan_root(&install).unwrap().sessions.len(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_sessions_and_roots_are_not_read() {
        let sample = Sample::new();
        let outside = Sample::new();
        outside.write(HEADER);
        fs::remove_dir_all(sample.0.join("sessions")).unwrap();
        std::os::unix::fs::symlink(outside.0.join("sessions"), sample.0.join("sessions")).unwrap();
        assert!(PiAdapter::scan_root(&sample.install())
            .unwrap()
            .sessions
            .is_empty());
        let link = sample.0.join("linked-root");
        std::os::unix::fs::symlink(&outside.0, &link).unwrap();
        let mut install = sample.install();
        install.data_roots = vec![link.display().to_string()];
        assert!(PiAdapter::scan_root(&install).unwrap().sessions.is_empty());
    }
}
