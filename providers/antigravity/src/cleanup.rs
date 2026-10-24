//! Coordinated Antigravity session cleanup (`Start.md` §12.4, §16, §18).
//! Preserve complete pre-images before editing a private index copy; never
//! trash a shared index or touch preserved brain artifacts.
use super::*;
use agenttidy_infrastructure::{
    atomic_file::{create_private_directory, create_private_file, replace_from_file},
    fs_probe::{file_identity, is_single_link_file},
    paths::{is_within, safe_canonicalize},
    sqlite::MutableSqliteDb,
    trash::{move_paths_to_trash, TrashOutcome},
};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const MAP: &str = "agyhub_summaries_proto.pb";
fn normalized(sql: &str) -> String {
    sql.trim().trim_end_matches(';').trim().to_owned()
}
pub(super) fn verify_schema(db: &ReadOnlyDb) -> anyhow::Result<()> {
    for (name, expected) in [
        (
            "conversation_summaries",
            include_str!("../../../fixtures/antigravity/index/schema.sql"),
        ),
        (
            "conversations_fts",
            include_str!("../../../fixtures/antigravity/index/fts-schema.sql"),
        ),
        (
            "search_schema_version",
            include_str!("../../../fixtures/antigravity/index/search-schema.sql"),
        ),
    ] {
        let actual: String = db.connection().query_row(
            "SELECT sql FROM sqlite_master WHERE name=?1",
            [name],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            normalized(&actual) == normalized(expected),
            "unsupported Antigravity {name} schema"
        );
    }
    let version: i64 =
        db.connection()
            .query_row("SELECT version FROM search_schema_version", [], |r| {
                r.get(0)
            })?;
    anyhow::ensure!(
        version == 3,
        "unsupported Antigravity search schema version"
    );
    let mut stmt = db
        .connection()
        .prepare("SELECT type,name FROM sqlite_master WHERE type IN ('table','trigger')")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (kind, name) = row?;
        anyhow::ensure!(
            kind != "trigger"
                && [
                    "conversation_summaries",
                    "search_schema_version",
                    "conversations_fts",
                    "conversations_fts_data",
                    "conversations_fts_idx",
                    "conversations_fts_content",
                    "conversations_fts_docsize",
                    "conversations_fts_config"
                ]
                .contains(&name.as_str()),
            "unknown Antigravity index table/trigger"
        );
    }
    Ok(())
}
fn preconditions() -> Vec<CleanupPrecondition> {
    [
        (
            CleanupPreconditionKind::PathExists,
            "complete session consistency set still exists",
        ),
        (
            CleanupPreconditionKind::PathUnchanged,
            "all state files remain inside the detected root",
        ),
        (
            CleanupPreconditionKind::IdentityUnchanged,
            "state file identities unchanged",
        ),
        (
            CleanupPreconditionKind::SizeUnchanged,
            "complete state sizes unchanged",
        ),
        (
            CleanupPreconditionKind::MtimeUnchanged,
            "complete state mtimes unchanged",
        ),
        (
            CleanupPreconditionKind::NoAgentRunning,
            "Antigravity and its language server are stopped",
        ),
        (
            CleanupPreconditionKind::SessionInactive,
            "session is archived/inactive with no dependent sessions",
        ),
        (
            CleanupPreconditionKind::SchemaUnchanged,
            "verified index and trajectory formats unchanged",
        ),
        (
            CleanupPreconditionKind::NoSymlinkOutsideRoot,
            "no links or hard-link aliases in the consistency set",
        ),
    ]
    .into_iter()
    .map(|(kind, description)| CleanupPrecondition {
        kind,
        description: description.into(),
    })
    .collect()
}
fn safe_paths(paths: &[PathBuf], root: &Path) -> anyhow::Result<()> {
    let canonical_root = safe_canonicalize(root)?;
    for path in paths {
        anyhow::ensure!(
            is_single_link_file(path) && is_within(&canonical_root, &safe_canonicalize(path)?),
            "state path is linked or escapes its root"
        );
        for ancestor in path.ancestors().take_while(|p| *p != root) {
            anyhow::ensure!(
                !fs::symlink_metadata(ancestor)?.file_type().is_symlink(),
                "linked state ancestor"
            );
        }
        if path.extension().is_some_and(|ext| ext == "db") {
            for suffix in ["-wal", "-shm"] {
                anyhow::ensure!(!PathBuf::from(format!("{}{suffix}",path.display())).try_exists().unwrap_or(true),"SQLite sidecars remain; close Antigravity normally to checkpoint before cleanup");
            }
        }
    }
    Ok(())
}
pub(super) fn units(snapshot: &AgentSnapshot) -> anyhow::Result<Vec<CleanupUnit>> {
    let root = AntigravityAdapter::root(&snapshot.installation)?;
    let mut result = vec![];
    for session in &snapshot.sessions {
        if session.metadata["body_verified"] != true
            || session.metadata["parent_conversation_id"] != ""
            || session.metadata["has_session_dependents"] != false
        {
            continue;
        }
        let resources: Vec<_> = snapshot
            .resources
            .iter()
            .filter(|r| {
                r.session_id.as_ref() == Some(&session.id) && r.kind == ResourceKind::Session
            })
            .collect();
        if resources.is_empty()
            || resources
                .iter()
                .any(|r| r.ownership != Ownership::Exclusive)
        {
            continue;
        }
        let mut paths = vec![root.join(INDEX)];
        if regular(&root.join(MAP)) {
            paths.push(root.join(MAP));
        }
        let mut owned = vec![];
        for resource in &resources {
            if let ResourceLocator::File { path } = &resource.locator {
                owned.push(PathBuf::from(path));
            }
        }
        owned.sort();
        paths.extend(owned);
        let metadata = fs::symlink_metadata(&paths[0])?;
        let Some(identity) = file_identity(&paths[0]) else {
            continue;
        };
        let Some(mtime_ms) = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_millis()).ok())
        else {
            continue;
        };
        result.push(CleanupUnit {
            id: format!("{}:session:{}", snapshot.installation.id, session.id.0),
            provider: snapshot.installation.provider.clone(),
            installation_id: snapshot.installation.id.clone(),
            kind: CleanupUnitKind::ProviderOperation,
            resources: resources
                .iter()
                .map(|r| ResourceRef {
                    id: r.id.clone(),
                    kind: r.kind,
                })
                .collect(),
            estimated_reclaimable_bytes: resources.iter().map(|r| r.size.logical_bytes).sum(),
            fingerprint: ResourceFingerprint {
                path: paths[0].clone(),
                identity: FileIdentity {
                    volume: identity.volume,
                    index: identity.index,
                },
                size_bytes: metadata.len(),
                mtime_ms,
            },
            locator: CleanupLocator::FileSet { paths },
            preconditions: preconditions(),
            operation: CleanupOperation {
                action: CleanupAction::ProviderOperation,
            },
            resource_kind: ResourceKind::Session,
        });
    }
    Ok(result)
}
pub(super) fn validate(
    unit: &CleanupUnit,
    installation: &AgentInstallation,
) -> anyhow::Result<Vec<CleanupPrecondition>> {
    anyhow::ensure!(
        installation.provider.as_str() == ProviderId::ANTIGRAVITY
            && unit.provider == installation.provider
            && unit.installation_id == installation.id,
        "foreign Antigravity unit"
    );
    let root = AntigravityAdapter::root(installation)?;
    let CleanupLocator::FileSet { paths } = &unit.locator else {
        anyhow::bail!("Antigravity operation requires its complete consistency set");
    };
    anyhow::ensure!(
        paths.first() == Some(&root.join(INDEX)) && paths.len() >= 2,
        "incomplete Antigravity state set"
    );
    safe_paths(paths, root)?;
    verify_schema(&AntigravityAdapter::open(root)?)?;
    let snapshot = AntigravityAdapter::scan_root(installation, &ScanOptions::default())?;
    anyhow::ensure!(
        !snapshot
            .problems
            .values()
            .any(|p| matches!(p, ScanProblem::Error { .. })),
        "scan has unresolved errors"
    );
    let candidates = units(&snapshot)?;
    anyhow::ensure!(
        candidates.iter().any(|candidate| candidate == unit),
        "Antigravity unit no longer matches the verified live session"
    );
    if regular(&root.join(MAP)) {
        state::remove_summary(&fs::read(root.join(MAP))?, &session_id(unit, installation)?)?;
    }
    Ok(preconditions())
}
fn session_id(unit: &CleanupUnit, installation: &AgentInstallation) -> anyhow::Result<String> {
    let id = unit
        .id
        .strip_prefix(&format!("{}:session:", installation.id))
        .ok_or_else(|| anyhow::anyhow!("invalid operation id"))?;
    anyhow::ensure!(valid_id(id), "invalid operation session id");
    Ok(id.into())
}
fn hash(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut block = [0; 64 * 1024];
    loop {
        let count = file.read(&mut block)?;
        if count == 0 {
            break;
        }
        digest.update(&block[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Member {
    original: PathBuf,
    backup: PathBuf,
    sha256: String,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    version: String,
    provider: String,
    root: PathBuf,
    session_id: String,
    members: Vec<Member>,
}
fn private_parent(path: &Path) -> anyhow::Result<()> {
    if !path.try_exists()? {
        create_private_directory(path)?;
    }
    anyhow::ensure!(
        fs::symlink_metadata(path)?.is_dir()
            && !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "recovery directory is redirected"
    );
    Ok(())
}
fn outcome(item: &CleanupItem, result: CleanupOutcome, reason: String) -> CleanupItemOutcome {
    CleanupItemOutcome {
        unit_id: item.unit_id.clone(),
        outcome: result,
        reclaimed_bytes: 0,
        reason: Some(reason),
    }
}

pub(super) fn execute(
    item: &CleanupItem,
    installation: &AgentInstallation,
) -> anyhow::Result<CleanupItemOutcome> {
    let Some(home) = agenttidy_infrastructure::paths::home_dir() else {
        return Ok(outcome(
            item,
            CleanupOutcome::Failed,
            "Cannot resolve recovery directory".into(),
        ));
    };
    let base = home.join(".agenttidy");
    private_parent(&base)?;
    let backup = base.join("backup");
    private_parent(&backup)?;
    execute_at(item, installation, &backup)
}

/// Testable entry point: production uses ~/.agenttidy/backup; acceptance uses
/// its own private temp folder, never a real user's sessions or recovery points.
fn execute_at(
    item: &CleanupItem,
    installation: &AgentInstallation,
    backup_base: &Path,
) -> anyhow::Result<CleanupItemOutcome> {
    let snapshot = AntigravityAdapter::scan_root(installation, &ScanOptions::default())?;
    let candidates = units(&snapshot)?;
    let Some(unit) = candidates.iter().find(|u| u.id == item.unit_id) else {
        return Ok(outcome(
            item,
            CleanupOutcome::Skipped,
            "Session is no longer cleanup eligible".into(),
        ));
    };
    anyhow::ensure!(
        item.action == CleanupAction::ProviderOperation
            && item.provider == installation.provider
            && item.installation_id == installation.id
            && item.locator == unit.locator
            && item.fingerprint == unit.fingerprint,
        "forged or stale Antigravity operation"
    );
    validate(unit, installation)?;
    anyhow::ensure!(!AntigravityAdapter::running(), "Antigravity is running");
    let root = AntigravityAdapter::root(installation)?;
    let id = session_id(unit, installation)?;
    let paths: Vec<_> = unit.locator.paths().into_iter().cloned().collect();
    let backup = backup_base.join(format!(
        "antigravity-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    create_private_directory(&backup)?;
    let mut manifest = Manifest {
        version: "agenttidy.antigravity-recovery.v1".into(),
        provider: ProviderId::ANTIGRAVITY.into(),
        root: safe_canonicalize(root)?,
        session_id: id.clone(),
        members: vec![],
    };
    for (index, path) in paths.iter().enumerate() {
        let before = hash(path)?;
        let destination = backup.join(format!("{index}.preimage"));
        let mut input = fs::File::open(path)?;
        let mut output = create_private_file(&destination)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        anyhow::ensure!(
            before == hash(path)? && before == hash(&destination)?,
            "source changed while backing up"
        );
        manifest.members.push(Member {
            original: path.clone(),
            backup: destination,
            sha256: before,
        });
    }
    let manifest_path = backup.join("manifest.json");
    let mut receipt = create_private_file(&manifest_path)?;
    receipt.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    receipt.sync_all()?;
    let mut changed = false;
    let operation = (|| -> anyhow::Result<()> {
        let mut copy = MutableSqliteDb::open_mutable_on_temp_copy(&manifest.members[0].backup)?;
        let transaction = copy.connection_mut().transaction()?;
        transaction.execute(
            "DELETE FROM conversations_fts WHERE conversation_id=?1",
            [&id],
        )?;
        anyhow::ensure!(
            transaction.execute(
                "DELETE FROM conversation_summaries WHERE conversation_id=?1",
                [&id]
            )? == 1,
            "target index row disappeared"
        );
        transaction.commit()?;
        anyhow::ensure!(
            !copy
                .connection()
                .prepare("PRAGMA foreign_key_check")?
                .exists([])?,
            "index foreign key check failed"
        );
        let prepared = backup.join("index.after.sqlite");
        copy.connection()
            .execute("VACUUM INTO ?1", [prepared.to_string_lossy().as_ref()])?;
        let check = ReadOnlyDb::open(&prepared)?;
        anyhow::ensure!(
            check.quick_check_ok()?,
            "prepared index integrity check failed"
        );
        drop(check);
        let map = root.join(MAP);
        let mut prepared_map = None;
        if let Some(member) = manifest.members.iter().find(|m| m.original == map) {
            let bytes = state::remove_summary(&fs::read(&member.backup)?, &id)?;
            let path = backup.join("summaries.after.pb");
            let mut file = create_private_file(&path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            prepared_map = Some(path);
        }
        // Re-prove complete source hashes and writer absence immediately before
        // the first write. All subsequent errors retain the full recovery set.
        safe_paths(&paths, root)?;
        anyhow::ensure!(
            !AntigravityAdapter::running(),
            "Antigravity started during backup"
        );
        for member in &manifest.members {
            anyhow::ensure!(
                hash(&member.original)? == member.sha256,
                "source changed after preview/backup"
            );
        }
        changed = true;
        replace_from_file(&prepared, &root.join(INDEX))?;
        if let Some(prepared) = prepared_map {
            anyhow::ensure!(
                !AntigravityAdapter::running(),
                "Antigravity started during commit"
            );
            replace_from_file(&prepared, &map)?;
        }
        anyhow::ensure!(
            !AntigravityAdapter::running(),
            "Antigravity started before session Trash"
        );
        let owned: Vec<_> = paths
            .iter()
            .filter(|p| **p != root.join(INDEX) && **p != map)
            .cloned()
            .collect();
        let results = move_paths_to_trash(&owned);
        anyhow::ensure!(
            results
                .iter()
                .all(|(_, result)| matches!(result, Ok(TrashOutcome::Removed))),
            "session Trash incomplete: {results:?}"
        );
        Ok(())
    })();
    let status = if operation.is_ok() {
        CleanupOutcome::Removed
    } else {
        CleanupOutcome::Failed
    };
    let reason=match operation {Ok(())=>format!("Coordinated session cleanup completed; complete recovery manifest: {}. Brain artifacts preserved. Trash/backups retain disk space.",manifest_path.display()),Err(error)=>format!("Coordinated cleanup failed (source changes may have started: {changed}): {error}. Complete recovery manifest: {}. Restore the full set only while Antigravity is closed and before newer work.",manifest_path.display())};
    Ok(outcome(item, status, reason))
}
