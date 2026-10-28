//! Size aggregation with hard-link deduplication (`Start.md` §8).
//!
//! The rules §8 fixes for space accounting:
//! - hard links are counted **once** (by file identity, not by path);
//! - a file whose identity could not be read is still counted, but the
//!   aggregate records it — an unidentifiable file cannot be *proven*
//!   to be a duplicate, so dedup must fail toward counting, never
//!   toward dropping;
//! - allocated size is only reported when every contribution was
//!   measurable (partial sums would be fake precision).
//!
//! The input is [`crate::fs_probe::EntryInfo`] from a non-following
//! walk; nested directories are naturally counted once because the
//! walker yields each entry exactly once.

use std::collections::HashSet;

use crate::fs_probe::{EntryInfo, EntryKind, FileIdentity};

/// Accumulates file sizes, deduplicating hard links by identity.
#[derive(Debug)]
pub struct UsageAccumulator {
    seen: HashSet<FileIdentity>,
    /// Bytes counted (deduped) — logical file lengths.
    logical_bytes: u64,
    /// Sum of on-disk allocated bytes; `None` once any contribution
    /// was unmeasurable (§8 no fake precision).
    allocated_bytes: Option<u64>,
    /// Hard-link duplicates that were skipped (they share an identity
    /// already counted). Reported for diagnostics.
    hardlink_duplicates_skipped: u64,
    /// Files counted whose identity was unavailable — they *could* be
    /// duplicates of something already counted, so the aggregate is a
    /// possible over-count. Surfaced, not hidden.
    unidentifiable_files: u64,
    /// Total files offered to the accumulator.
    files_seen: u64,
}

impl Default for UsageAccumulator {
    /// Hand-written because the derived default would start
    /// `allocated_bytes` at `None`, and the first accumulation
    /// `opt_add_for_usage(None, Some(x))` would collapse to `None`
    /// again — so the allocated total could never leave `None` no
    /// matter how many measurable files were added (§8 "allocated
    /// size" would never be reported). Starting at `Some(0)` keeps the
    /// intended rule intact: the total stays known while every
    /// contribution is measurable, and drops to `None` as soon as one
    /// is not.
    fn default() -> Self {
        Self {
            seen: HashSet::new(),
            logical_bytes: 0,
            allocated_bytes: Some(0),
            hardlink_duplicates_skipped: 0,
            unidentifiable_files: 0,
            files_seen: 0,
        }
    }
}

impl UsageAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one walked file entry. Only [`EntryKind::File`] entries are
    /// meaningful input; links must be excluded by the caller (their
    /// targets are out of scope per §16.1, and the link itself is not
    /// agent data worth counting).
    pub fn add_file(&mut self, entry: &EntryInfo) {
        debug_assert!(
            matches!(entry.kind, EntryKind::File),
            "UsageAccumulator::add_file is for files; got {:?} at {}",
            entry.kind,
            entry.path.display()
        );
        self.files_seen += 1;

        match entry.identity {
            // Second+ hard link to a file already counted: the bytes
            // are on disk exactly once (§8), do not add them again.
            Some(id) if self.seen.contains(&id) => {
                self.hardlink_duplicates_skipped += 1;
            }
            Some(id) => {
                self.seen.insert(id);
                self.logical_bytes += entry.logical_len;
                self.allocated_bytes = self.allocated_bytes.opt_add_for_usage(entry.allocated_len);
            }
            // Identity unavailable (locked file, race): count it —
            // dropping it could under-report agent data — and surface
            // the uncertainty for diagnostics.
            None => {
                self.unidentifiable_files += 1;
                self.logical_bytes += entry.logical_len;
                self.allocated_bytes = self.allocated_bytes.opt_add_for_usage(entry.allocated_len);
            }
        }
    }

    /// Fold the accumulator into the final usage figure.
    pub fn finish(self) -> DiskUsage {
        DiskUsage {
            logical_bytes: self.logical_bytes,
            allocated_bytes: self.allocated_bytes,
            files_seen: self.files_seen,
            hardlink_duplicates_skipped: self.hardlink_duplicates_skipped,
            unidentifiable_files: self.unidentifiable_files,
        }
    }
}

/// Local `Option<u64>` addition mirroring the no-partial-sums stance of
/// `agenttidy_core::size::SizeInfo::merge` (kept private here to avoid
/// growing the core API for an internal helper).
trait OptAddUsage {
    fn opt_add_for_usage(self, other: Option<u64>) -> Option<u64>;
}

impl OptAddUsage for Option<u64> {
    fn opt_add_for_usage(self, other: Option<u64>) -> Option<u64> {
        match (self, other) {
            (Some(a), Some(b)) => Some(a + b),
            // An unmeasured contribution makes the whole sum unknown —
            // never silently present a partial total as complete.
            _ => None,
        }
    }
}

/// Final per-scope disk usage (a provider's translation layer maps this
/// onto [`agenttidy_core::size::SizeInfo`]; core types stay untouched).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskUsage {
    /// Deduplicated sum of file logical lengths.
    pub logical_bytes: u64,
    /// On-disk allocated bytes, only when fully measured.
    pub allocated_bytes: Option<u64>,
    /// Number of file entries offered.
    pub files_seen: u64,
    /// Hard-link entries not counted (§8 dedup).
    pub hardlink_duplicates_skipped: u64,
    /// Files counted without a provable identity (possible over-count).
    pub unidentifiable_files: u64,
}

impl Default for DiskUsage {
    /// An empty scope has a *known* allocated size of zero — there is
    /// no unmeasured contribution to make it unknown. The derived
    /// default would report `None` and disagree with an empty
    /// accumulator, which reports `Some(0)`.
    fn default() -> Self {
        Self {
            logical_bytes: 0,
            allocated_bytes: Some(0),
            files_seen: 0,
            hardlink_duplicates_skipped: 0,
            unidentifiable_files: 0,
        }
    }
}

/// Convenience: aggregate the `File` entries of one walk outcome.
#[must_use]
pub fn usage_of_entries(entries: &[EntryInfo]) -> DiskUsage {
    let mut acc = UsageAccumulator::new();
    for entry in entries.iter().filter(|e| e.kind == EntryKind::File) {
        acc.add_file(entry);
    }
    acc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agenttidy-usage-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file_entry(path: PathBuf, len: u64, id: Option<FileIdentity>) -> EntryInfo {
        EntryInfo {
            path,
            kind: EntryKind::File,
            logical_len: len,
            allocated_len: Some(len), // tests do not care about allocation
            identity: id,
        }
    }

    #[test]
    fn hardlinks_count_once() {
        let id = FileIdentity {
            volume: 1,
            index: 7,
        };
        let mut acc = UsageAccumulator::new();
        acc.add_file(&file_entry(PathBuf::from("/x/a"), 100, Some(id)));
        acc.add_file(&file_entry(PathBuf::from("/x/b"), 100, Some(id)));
        acc.add_file(&file_entry(PathBuf::from("/x/c"), 50, None));

        let usage = acc.finish();
        // 100 + 50: the duplicate hard link is not counted twice (§8).
        assert_eq!(usage.logical_bytes, 150);
        assert_eq!(usage.hardlink_duplicates_skipped, 1);
        assert_eq!(usage.files_seen, 3);
        // One file had no identity — surfaced, still counted.
        assert_eq!(usage.unidentifiable_files, 1);
    }

    #[test]
    fn unmeasured_allocation_drops_to_none() {
        let mut acc = UsageAccumulator::new();
        acc.add_file(&file_entry(
            PathBuf::from("/x/a"),
            10,
            Some(FileIdentity {
                volume: 1,
                index: 1,
            }),
        ));
        let mut unknown_alloc = file_entry(
            PathBuf::from("/x/b"),
            20,
            Some(FileIdentity {
                volume: 1,
                index: 2,
            }),
        );
        unknown_alloc.allocated_len = None;
        acc.add_file(&unknown_alloc);

        let usage = acc.finish();
        assert_eq!(usage.logical_bytes, 30);
        // A partial allocation sum must not look complete (§8).
        assert_eq!(usage.allocated_bytes, None);
    }

    #[test]
    fn real_files_via_walk_dedup_hardlinks() {
        let root = temp_root("real");
        let a = root.join("a.txt");
        std::fs::write(&a, vec![0u8; 128]).unwrap();
        let b = root.join("b.txt");
        std::fs::hard_link(&a, &b).expect("hard link");

        let walk = crate::fs_probe::walk_tree(&root);
        let usage = usage_of_entries(&walk.entries);

        assert_eq!(usage.logical_bytes, 128, "two links, one physical file");
        assert_eq!(usage.hardlink_duplicates_skipped, 1);
        // Real platform identities were available.
        assert_eq!(usage.unidentifiable_files, 0);
        assert_eq!(usage.files_seen, 2);
    }

    #[test]
    fn empty_usage_is_zero() {
        assert_eq!(usage_of_entries(&[]), DiskUsage::default());
    }

    #[test]
    fn fully_measurable_files_report_the_allocated_total() {
        // Regression: `allocated_bytes` used to stay `None` forever,
        // because the accumulator started at `None` and the first
        // `opt_add_for_usage(None, Some(_))` collapsed back to `None`.
        // A fully measurable scope must report its allocated total.
        let mut acc = UsageAccumulator::new();
        acc.add_file(&file_entry(
            PathBuf::from("/x/a"),
            100,
            Some(FileIdentity {
                volume: 1,
                index: 1,
            }),
        ));
        acc.add_file(&file_entry(
            PathBuf::from("/x/b"),
            250,
            Some(FileIdentity {
                volume: 1,
                index: 2,
            }),
        ));

        let usage = acc.finish();
        assert_eq!(usage.logical_bytes, 350);
        assert_eq!(usage.allocated_bytes, Some(350));
    }

    #[test]
    fn a_single_unmeasured_file_drops_the_allocated_total_to_none() {
        // The complementary rule: one unmeasurable contribution must
        // make the whole allocated total unknown (§8, no fake
        // precision) — but it must not make it unknown *by default*.
        let mut acc = UsageAccumulator::new();
        acc.add_file(&file_entry(
            PathBuf::from("/x/a"),
            100,
            Some(FileIdentity {
                volume: 1,
                index: 1,
            }),
        ));
        let mut unmeasured = file_entry(
            PathBuf::from("/x/b"),
            20,
            Some(FileIdentity {
                volume: 1,
                index: 2,
            }),
        );
        unmeasured.allocated_len = None;
        acc.add_file(&unmeasured);

        let usage = acc.finish();
        assert_eq!(usage.logical_bytes, 120);
        assert_eq!(usage.allocated_bytes, None);
    }
}
