# Provider DB Records (`provider-operation`)

Status: **cleanup enabled (Phase 7 beta)** for the Codex provider's
archived sessions only. Other providers are not enabled — see
[`workspace-cleanup.md`](./workspace-cleanup.md) for the general
contract and [`session-transcript-fileset.md`](./session-transcript-fileset.md)
for the parallel `file-set` resource type (Claude Code, WorkBuddy).

This document covers the `provider-operation` resource type: one agent
session's coordinated database rows plus its on-disk artifacts, treated
as one atomic cleanup unit (`Start.md` §9.5). The general cleanup
execution contract — risk vocabulary, §12.2 revalidation triggers, two
confirmations, audit log — lives in
[`workspace-cleanup.md`](./workspace-cleanup.md) and applies unchanged.

## What forms a unit

One unit per **recognized, exclusively-owned session** whose complete
state lives across one database and several on-disk files (the *consistency
set*). For Codex archived sessions the consistency set is:

| Member | Path | Role |
| --- | --- | --- |
| Thread row | `state_5.sqlite` `threads.id = <session-id>` | Authoritative lifecycle flag (`archived`); join key for every other member |
| History rows | `thread_history_1.sqlite.thread_items`, `thread_history_1.thread_turns` (both `thread_id = <session-id>`) | Searchable projection of the rollout body — leaves rows behind if deleted out of order |
| Spawn edges | `state_5.sqlite.thread_spawn_edges` (`parent_thread_id` or `child_thread_id = <session-id>`) | Subagent relationship — orphaned edges mis-state the parent/child tree |
| Rollout file | `archived_sessions/rollout-<session-id>.jsonl` (or `sessions/...` for the live twin) | Primary transcript the user already sees in the UI |
| Index line | `session_index.jsonl` (one line with `id = <session-id>`) | UI index line — leaving it behind shows a tombstone for a thread that no longer exists |
| Attachments | `attachments/<session-id>/` (per-thread blobs) | Optional; only included when present on disk |

The provider's `build_cleanup_units` reads `state_5.sqlite.threads` (the
authoritative source per `Start.md` §2.4 + §12.4) and produces exactly
one `CleanupUnit` per archived-and-exclusively-owned session; the
*other* members are listed as `CleanupUnit::dependencies` and the
executor touches them as one transaction. The exact set is what the
read-only scan already measured and displayed — the cleanup unit never
adds or discovers paths at cleanup time.

**Never forms a unit** (stays report-only, §3.2 + §7.3):

- sessions whose first line `session_meta` is missing or doesn't match
  the rollout path (`Unknown` resource, `Ownership::Unknown`);
- `Unknown`-kind resources and anything under `include_unknown`;
- live (non-archived) sessions — `state_5.threads.archived = 0` is a
  clear *unknown* signal for whether the user is done with the thread
  (red line #5: archive ≠ permission);
- sessions whose rollout file is in `sessions/` but `threads.archived = 1`
  (or the inverse) — the file ↔ DB disagreement marks them
  `SessionLifecycle::Unknown` at scan time and they are reported as a
  problem, not unit candidates;
- default workspaces (`~/Documents/Codex/...`) — user-mixed per
  `Start.md` §2.3, never a unit regardless of contents.

## Operation and recovery

- Action is **`provider-operation` only** (mapped to
  `CleanupAction::ProviderOperation`). The provider executes an internal
  sequence: **backup → BEGIN IMMEDIATE → coordinated DELETE →
  PRAGMA wal_checkpoint → foreign_key_check → atomic replace**. The OS
  trash is **not** involved — red line #8 (`Start.md` §12.4) forbids
  describing database row deletion as "Move to Trash".
- Recovery is performed **from the backup file** written before execute:
  `~/.agenttidy/backup/state_5-<plan-id>-<unit-id>-<ts>.sqlite` (and
  `-wal` / `-shm` sidecars). The user runs SQLite to restore manually
  (`sqlite3 state_5.sqlite ".restore '<backup>'"`); AgentTidy shows
  no in-app Undo (`Start.md` §16.4).
- The execution is **transactional** but **not globally atomic**: if
  step ③ fails partway through, the rolled-back temp-copy DB is
  discarded and the agent's original DB is untouched; if step ④ (the
  atomic rename back to the agent's source path) fails, the executor
  restores from the backup file and reports `Failed` with the verbatim
  error (§12.3 — partial failure is reported honestly, never as
  success). Re-running is safe: rows already deleted count as
  idempotent no-ops (`Skipped`), not fresh success.
- No `Move to Trash` text ever appears in the UI, audit log, or
  troubleshooting output for this resource type.

### Native CLI bridge (Codex only)

If Codex exposes a documented thread-deletion CLI
(`codex thread delete <id>`, `codex thread archive --purge`, or
equivalent — see [`docs/providers/codex.md`](../providers/codex.md) for
the investigation result), the Application layer prefers it: one
out-of-process invocation per unit, the provider's own binary is
responsible for the §12.4 discipline (backup, transaction, FK check).
The Application layer records the exit code, stdout and stderr verbatim
in the audit log; a non-zero exit falls back to the §12.4 self-
implementation below.

### §12.4 self-implementation (fallback / no native CLI)

When the native CLI is unavailable, fails, or its behaviour is
undocumented, the executor runs the §12.4 sequence directly on a
private temp copy (see `infrastructure::sqlite::MutableSqliteDb`):

1. `inspect()` re-check (`schema_versions`, `journal_modes`,
   `agent_running`); abort if any of them drifted.
2. `~/.codex/thread-writer-locks/` is empty.
3. `backup_sqlite_with_sidecars` writes a `VACUUM INTO` snapshot of
   `state_5.sqlite` (+ `-wal` + `-shm`) and of `thread_history_1.sqlite`
   to `~/.agenttidy/backup/`; the backup paths are recorded in the
   audit log's `reason` field.
4. `open_mutable_on_temp_copy` opens a writable copy of `state_5.sqlite`
   in a private temp dir; the agent's source file is never opened for
   writing (red line #13).
5. `BEGIN IMMEDIATE` transaction — `DELETE FROM threads WHERE id = ?`,
   `DELETE FROM thread_history_1.thread_items WHERE thread_id = ?`,
   `DELETE FROM thread_history_1.thread_turns WHERE thread_id = ?`,
   `DELETE FROM state_5.thread_attachments WHERE thread_id = ?`,
   `DELETE FROM state_5.thread_spawn_edges WHERE parent_thread_id = ?
   OR child_thread_id = ?` — `COMMIT`.
6. `PRAGMA wal_checkpoint(TRUNCATE)`, then `PRAGMA quick_check` and
   `PRAGMA foreign_key_check`. Any failure ⇒ restore from backup,
   report `Failed`.
7. `fs::rename` the mutated temp DB over the agent's source DB;
   unlink stale `-wal` / `-shm`.
8. `reclaimed_bytes` = `(pre_vacuum_pages - post_vacuum_pages) *
   page_size` from a follow-up `VACUUM`. The on-disk rollout JSONL
   remains in `archived_sessions/` (it is the user-visible archive,
   not free space); only DB rows + projection rows + index line are
   mutated.

## Pre-conditions (declared at plan time, checked before execute)

The provider declares these typed §12.2 triggers on every unit; the
revalidator reports the first failing one per item:

| Trigger | Meaning for this resource type |
| --- | --- |
| `path-exists` | `state_5.sqlite` still exists and is readable |
| `path-unchanged` | `state_5.sqlite` canonicalizes to the scanned location (§16.1) |
| `identity-unchanged` | `state_5.sqlite` file identity unchanged |
| `size-unchanged` | `state_5.sqlite` size unchanged (rough drift check) |
| `mtime-unchanged` | `state_5.sqlite` mtime unchanged — doubles as the agent-active trigger (a running Codex writes its DB) |
| `no-agent-running` | No Codex process detected at execute time (§16.2) |
| `session-inactive` | Owning session lifecycle is still `Archived` (file ↔ DB agree) |
| `schema-unchanged` | `state_5.sqlite` `user_version` + `threads` schema unchanged from the snapshot |
| `writer-lock-absent` | `~/.codex/thread-writer-locks/` is empty |

Known bound (documented limitation of the beta): drift *inside* the
projection rows (`thread_history_1`) is not individually fingerprinted
per row — it is covered by `no-agent-running` + `mtime-unchanged` and
the executor's transaction-level `foreign_key_check`.

## Path safety (§16.1)

`validate_cleanup_unit` runs at plan build and rejects a unit unless:

- the `database` path in the `CleanupLocator::DbRow` is non-empty and
  free of reserved components (`paths::has_reserved_component`);
- the database canonicalizes to a path inside the installation's data
  roots (no symlink escapes);
- the `record_id` is non-empty and matches the snapshot's session id
  (no fabricated identifiers);
- for the Codex self-implementation path: the rollout JSONL whose
  stem equals `record_id` still exists at `archived_sessions/` (the
  executor moves no JSONL, but a missing twin is a §3.2 "do not act
  on what we don't understand" signal — drop the unit).

Deletion targets come from the scan snapshot only; user-typed paths
are never accepted (§16.1 red line).

## Concurrency (§12.1, §16.2)

Execution holds the cross-process operation guard
(`$HOME/.agenttidy/operations.lock`) for the whole batch: a second GUI
window or CLI instance cannot execute concurrently. A live Codex
process blocks execution at both policy (`agent-running`) and
revalidation (`no-agent-running`) time. `~/.codex/thread-writer-locks/`
must be empty at execute time — the `writer-lock-absent` precondition
enforces this; a non-empty lock dir is reported `Skipped` with the
verbatim count of held locks and the user is asked to retry.

## Audit log (§16.3)

One `agenttidy.audit.v1` row per item at
`$HOME/.agenttidy/operations.jsonl`: plan id, unit id, provider, the
`db-row` locator (database path + record id), action
`provider-operation`, outcome, estimated/actual reclaimed bytes and
the backup paths used for recovery. No message bodies, tokens, file
contents, or SQL statements are ever logged.