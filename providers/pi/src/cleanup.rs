//! Verified Pi single-transcript cleanup (`Start.md` §10, §12, §16, §18).
//! Native Pi deletes exactly one session file; never infer a project or sidecar
//! as part of that operation. Safety contract: docs/safety/pi-session-fileset.md.

use super::*;
use agenttidy_infrastructure::{
    fs_probe::file_identity,
    paths::{is_within, safe_canonicalize},
};

pub(super) fn native_filename(path: &Path, id: &str) -> bool {
    let valid_id = id.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && id.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    let suffix = format!("_{id}.jsonl");
    valid_id
        && path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| name.len() > suffix.len() && name.ends_with(&suffix))
}

fn preconditions() -> Vec<CleanupPrecondition> {
    [
        (
            CleanupPreconditionKind::PathExists,
            "Pi transcript still exists",
        ),
        (
            CleanupPreconditionKind::PathUnchanged,
            "Pi transcript stays inside its verified session root",
        ),
        (
            CleanupPreconditionKind::IdentityUnchanged,
            "Pi transcript identity unchanged",
        ),
        (
            CleanupPreconditionKind::SizeUnchanged,
            "Pi transcript size unchanged",
        ),
        (
            CleanupPreconditionKind::MtimeUnchanged,
            "Pi transcript mtime unchanged",
        ),
        (
            CleanupPreconditionKind::NoAgentRunning,
            "no Pi writer is running",
        ),
        (
            CleanupPreconditionKind::SessionInactive,
            "Pi session is still inactive",
        ),
        (
            CleanupPreconditionKind::NoSymlinkOutsideRoot,
            "Pi transcript has no linked ancestors or aliases",
        ),
    ]
    .into_iter()
    .map(|(kind, description)| CleanupPrecondition {
        kind,
        description: description.into(),
    })
    .collect()
}

fn safe_file(path: &Path, id: &str, installation: &AgentInstallation) -> anyhow::Result<()> {
    anyhow::ensure!(
        installation.provider.as_str() == ProviderId::PI,
        "foreign Pi installation"
    );
    let root = Path::new(
        installation
            .data_roots
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing Pi root"))?,
    );
    let sessions = installation
        .data_roots
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("sessions"));
    anyhow::ensure!(
        fs::symlink_metadata(root).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()),
        "Pi root is unavailable or linked"
    );
    anyhow::ensure!(
        fs::symlink_metadata(&sessions).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()),
        "Pi session root is unavailable or linked"
    );
    anyhow::ensure!(
        is_single_link_file(path) && native_filename(path, id),
        "Pi transcript is linked or has an unverified filename"
    );
    anyhow::ensure!(
        is_within(&safe_canonicalize(&sessions)?, &safe_canonicalize(path)?),
        "Pi transcript escapes its session root"
    );
    for ancestor in path.ancestors().take_while(|p| *p != sessions) {
        anyhow::ensure!(
            !fs::symlink_metadata(ancestor)?.file_type().is_symlink(),
            "Pi transcript has a linked ancestor"
        );
    }
    let meta = header(path)?;
    anyhow::ensure!(meta["id"].as_str() == Some(id), "Pi transcript id changed");
    Ok(())
}

pub(super) fn units(snapshot: &AgentSnapshot) -> anyhow::Result<Vec<CleanupUnit>> {
    let mut units = Vec::new();
    for resource in &snapshot.resources {
        if resource.kind != ResourceKind::Session || resource.ownership != Ownership::Exclusive {
            continue;
        }
        let (Some(session), ResourceLocator::File { path }) =
            (&resource.session_id, &resource.locator)
        else {
            continue;
        };
        let path = PathBuf::from(path);
        if safe_file(&path, session.as_str(), &snapshot.installation).is_err() {
            continue;
        }
        let Some(identity) = file_identity(&path) else {
            continue;
        };
        let metadata = fs::symlink_metadata(&path)?;
        let Some(mtime_ms) = metadata
            .modified()
            .ok()
            .and_then(millis)
            .and_then(|t| i64::try_from(t).ok())
        else {
            continue;
        };
        units.push(CleanupUnit {
            id: format!("{}:session:{}", snapshot.installation.id, session.0),
            provider: snapshot.installation.provider.clone(),
            installation_id: snapshot.installation.id.clone(),
            kind: CleanupUnitKind::FileSet,
            resources: vec![ResourceRef {
                id: resource.id.clone(),
                kind: ResourceKind::Session,
            }],
            estimated_reclaimable_bytes: metadata.len(),
            fingerprint: ResourceFingerprint {
                path: path.clone(),
                identity: FileIdentity {
                    volume: identity.volume,
                    index: identity.index,
                },
                size_bytes: metadata.len(),
                mtime_ms,
            },
            locator: CleanupLocator::FileSet { paths: vec![path] },
            preconditions: preconditions(),
            operation: CleanupOperation {
                action: CleanupAction::Trash,
            },
            resource_kind: ResourceKind::Session,
        });
    }
    Ok(units)
}

pub(super) fn validate(
    unit: &CleanupUnit,
    installation: &AgentInstallation,
) -> anyhow::Result<Vec<CleanupPrecondition>> {
    anyhow::ensure!(
        unit.provider.as_str() == ProviderId::PI && unit.installation_id == installation.id,
        "foreign Pi cleanup unit"
    );
    let CleanupLocator::FileSet { paths } = &unit.locator else {
        anyhow::bail!("Pi cleanup must use a single-file Trash unit");
    };
    anyhow::ensure!(
        paths.len() == 1 && unit.operation.action == CleanupAction::Trash,
        "Pi cleanup must trash exactly one transcript"
    );
    // Reconstruct membership rather than trusting a unit serialized by the UI.
    let snapshot = PiAdapter::scan_root(installation)?;
    let candidates = units(&snapshot)?;
    anyhow::ensure!(
        candidates.iter().any(|candidate| candidate == unit),
        "Pi unit no longer matches the verified live session"
    );
    Ok(preconditions())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::Sample;

    fn eligible(sample: &Sample) -> PathBuf {
        let original = sample.write(crate::tests::HEADER);
        let meta: serde_json::Value = serde_json::from_str(crate::tests::HEADER).unwrap();
        let path = original.with_file_name(format!(
            "2026-09-24T03-24-20-391Z_{}.jsonl",
            meta["id"].as_str().unwrap()
        ));
        fs::rename(original, &path).unwrap();
        path
    }
    #[tokio::test]
    async fn single_transcript_forms_a_reviewable_trash_unit_and_rejects_forgery() {
        let sample = Sample::new();
        let path = eligible(&sample);
        let installation = sample.install();
        let snapshot = PiAdapter::scan_root(&installation).unwrap();
        let candidate = units(&snapshot)
            .unwrap()
            .pop()
            .expect("verified native filename");
        assert_eq!(candidate.operation.action, CleanupAction::Trash);
        assert_eq!(
            candidate.locator,
            CleanupLocator::FileSet {
                paths: vec![path.clone()]
            }
        );
        assert!(validate(&candidate, &installation).is_ok());
        let mut forged = candidate.clone();
        if let CleanupLocator::FileSet { paths } = &mut forged.locator {
            paths.push(sample.0.join("auth.json"));
        }
        assert!(validate(&forged, &installation).is_err());
        fs::write(&path, format!("{}\n", crate::tests::HEADER)).unwrap();
        assert!(
            validate(&candidate, &installation).is_err(),
            "size/mtime drift must invalidate the unit"
        );
    }
    #[test]
    fn external_hardlink_and_duplicate_ids_cannot_become_units() {
        let sample = Sample::new();
        let path = eligible(&sample);
        fs::hard_link(&path, sample.0.join("external-alias")).unwrap();
        assert!(units(&PiAdapter::scan_root(&sample.install()).unwrap())
            .unwrap()
            .is_empty());
        fs::remove_file(sample.0.join("external-alias")).unwrap();
        fs::copy(&path, path.with_file_name("copy.jsonl")).unwrap();
        assert!(units(&PiAdapter::scan_root(&sample.install()).unwrap())
            .unwrap()
            .is_empty());
    }
}
