//! SQLite backup helper used by the §12.4 self-implemented
//! `ProviderOperation` cleanup path (`docs/safety/provider-operation.md`).
//!
//! Every Codex (or future provider) delete-then-replace sequence starts
//! with `backup_sqlite_with_sidecars`: a `VACUUM INTO` snapshot of the
//! live database including its committed WAL contents, written
//! under `~/.agenttidy/backup/<plan-id>-<unit-id>-<ts>.sqlite`. The
//! backup path is then recorded in the audit log's `reason` field so
//! recovery is possible without a separate restore UI (`Start.md`
//! §16.4 — recovery is from the SQLite backup, not from the OS trash).
//!
//! Why `VACUUM INTO` rather than `fs::copy`:
//!
//! - `VACUUM INTO` rewrites the database into a single file with no
//!   free-list pages, so the snapshot is self-contained and
//!   recovery does not need to chase WAL sidecars.
//! - It runs inside SQLite itself, so the snapshot is consistent
//!   even if a checkpoint is in progress.
//! - It returns a freshly created file; no truncation race with
//!   concurrent writers.
//!
//! The bundled SQLite supports `VACUUM INTO`; errors fail closed. A raw
//! copy fallback cannot guarantee a consistent WAL snapshot. Backups are
//! self-contained and must not be paired with copies of the source WAL.

use std::path::{Path, PathBuf};

/// Snapshot `src`, including committed WAL contents, into `backup_dir`.
/// Returns the absolute path of the backup's primary database file.
/// On success the caller can `mv` the backup to its eventual
/// destination (typically under
/// `~/.agenttidy/backup/<plan-id>-<unit-id>-<ts>.sqlite`).
///
/// `backup_dir` is created if needed. Existing recovery points are never
/// overwritten; callers must supply a unique stem for each attempt. The
/// legacy function name is retained, but WAL contents are folded into the
/// standalone database rather than copied as sidecars.
pub fn backup_sqlite_with_sidecars(
    src: &Path,
    backup_dir: &Path,
    file_stem: &str,
) -> anyhow::Result<PathBuf> {
    if !src.is_file() {
        anyhow::bail!("backup source does not exist: {}", src.display());
    }
    std::fs::create_dir_all(backup_dir)?;

    let backup_path = backup_dir.join(format!("{file_stem}.sqlite"));

    // Reserve a new name without clobbering an earlier recovery point.
    // Bundled SQLite supports VACUUM INTO; a failed consistent snapshot
    // must not silently degrade to a potentially torn file copy (§12.4).
    let reserved = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&backup_path)?;
    drop(reserved);
    if let Err(error) = run_vacuum_into(src, &backup_path) {
        std::fs::remove_file(&backup_path).ok();
        return Err(error.into());
    }

    // Integrity gate: refuse to return a snapshot that fails
    // `PRAGMA quick_check` — a corrupt backup would later load as
    // garbage on recovery.
    let conn = rusqlite::Connection::open_with_flags(
        &backup_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if !result.eq_ignore_ascii_case("ok") {
        anyhow::bail!(
            "backup at {} failed quick_check: {result}",
            backup_path.display()
        );
    }

    Ok(backup_path)
}

/// SQLite reads a consistent source snapshot (including committed WAL frames)
/// and writes a standalone backup. Binding the destination avoids quoting and
/// URI interpretation of arbitrary user paths (§16.1).
fn run_vacuum_into(src: &Path, backup_path: &Path) -> rusqlite::Result<()> {
    let conn =
        rusqlite::Connection::open_with_flags(src, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.execute("VACUUM INTO ?1", [backup_path.to_string_lossy().as_ref()])?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(backup_path)
        .and_then(|file| file.sync_all())
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agenttidy-backup-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        std::fs::remove_dir_all(dir).ok();
    }

    /// A happy-path backup on a freshly-created database must
    /// produce a file that round-trips through `quick_check`.
    #[test]
    fn backup_happy_path_writes_quick_checkable_file() {
        let src_dir = temp_dir("src");
        let backup_dir = temp_dir("backup");
        let src = src_dir.join("source.sqlite");
        {
            let conn = rusqlite::Connection::open(&src).unwrap();
            conn.execute("CREATE TABLE t (x INTEGER PRIMARY KEY)", [])
                .unwrap();
            conn.execute("INSERT INTO t VALUES (1)", []).unwrap();
        }
        let backup_path = backup_sqlite_with_sidecars(&src, &backup_dir, "snapshot")
            .expect("backup must succeed");
        assert!(backup_path.is_file());
        assert_eq!(backup_path, backup_dir.join("snapshot.sqlite"));
        // Verify the rows came through.
        let conn = rusqlite::Connection::open(&backup_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        cleanup(&src_dir);
        cleanup(&backup_dir);
    }

    /// A missing source fails before reserving or writing a recovery point.
    #[test]
    fn backup_rejects_nonexistent_source() {
        let backup_dir = temp_dir("backup-reject");
        let missing = backup_dir.join("nonexistent.sqlite");
        let error = backup_sqlite_with_sidecars(&missing, &backup_dir, "snapshot")
            .expect_err("missing source must be rejected");
        assert!(error.to_string().contains("does not exist"));
        cleanup(&backup_dir);
    }

    /// WAL-mode databases produce a standalone, quick-checkable backup;
    /// a separate live-WAL test keeps the source connection open.
    #[test]
    fn backup_produces_quick_checkable_primary_file() {
        let src_dir = temp_dir("src-wal");
        let backup_dir = temp_dir("backup-wal");
        let src = src_dir.join("source.sqlite");
        {
            let conn = rusqlite::Connection::open(&src).unwrap();
            conn.pragma_update(None, "journal_mode", "wal").unwrap();
            conn.execute("CREATE TABLE t (x)", []).unwrap();
            conn.execute("INSERT INTO t VALUES (1)", []).unwrap();
        }
        let backup_path = backup_sqlite_with_sidecars(&src, &backup_dir, "snapshot")
            .expect("backup must succeed");
        assert!(backup_path.is_file());
        // Verify the rows came through (covers the VACUUM INTO path).
        let conn = rusqlite::Connection::open(&backup_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        cleanup(&src_dir);
        cleanup(&backup_dir);
    }
    /// Keep the writer connection open so committed rows remain in the WAL.
    /// A raw copy of only the main file cannot satisfy this regression (§18.4).
    #[test]
    fn backup_includes_live_wal_and_preserves_existing_recovery_points() {
        let root = temp_dir("live-wal");
        let source = root.join("source.sqlite");
        let writer = rusqlite::Connection::open(&source).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
            CREATE TABLE records (id INTEGER); INSERT INTO records VALUES (42);",
            )
            .unwrap();
        let backup = backup_sqlite_with_sidecars(&source, &root, "recovery").unwrap();
        let reader = rusqlite::Connection::open(&backup).unwrap();
        let value: i64 = reader
            .query_row("SELECT id FROM records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, 42);
        drop(reader);
        let original = std::fs::read(&backup).unwrap();
        writer
            .execute("INSERT INTO records VALUES (99)", [])
            .unwrap();
        assert!(backup_sqlite_with_sidecars(&source, &root, "recovery").is_err());
        assert_eq!(std::fs::read(&backup).unwrap(), original);
        assert!(!root.join("recovery.sqlite-wal").exists());
        drop(writer);
        cleanup(&root);
    }
}
