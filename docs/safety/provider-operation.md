# Provider DB Records (`provider-operation`)

Status: **Blocked / read-only candidates** for Codex archived sessions.
Native deletion and direct database replacement are disabled until successful
native deletion and complete database/transcript/index/attachment recovery
have passed acceptance. SQLite backups alone do not establish full recovery.
The investigated `codex-cli 0.152.0` is checked during preview and Diagnostics;
CLI availability is independent of the recovery gate and cannot enable cleanup.

This is a `provider-operation`, never “Move to Trash”. The shared policy,
two confirmations, revalidation and audit contract lives in
[`workspace-cleanup.md`](./workspace-cleanup.md) (`Start.md` §12 / §16).

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

The scan links a rollout resource to its authoritative archived thread.
A unit carries `DbRow { database: state_5.sqlite, record_id }`; its
fingerprint uses the database's own identity, size and mtime. Preview
metadata evidence also covers the linked rollout, existing DB sidecars,
canonical paths, provider version, schema SQL, `user_version` and journal
mode. No transcript bodies are stored in the plan.

**Mac acceptance remains open:** on 2026-10-20 an isolated synthetic native
operation returned `failed to delete session`. Version/help admission is
not proof of a successful or completely recoverable delete. See
[`../testing/macos-scenarios.md`](../testing/macos-scenarios.md).

The consistency-set table describes the native provider's responsibility;
AgentTidy does not independently delete any of those members or claim a
cross-file atomic transaction. The native protocol still needs real-user
Trash/Recovery/Compatibility acceptance before Phase 7 is closed.

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

Current production behavior:

1. Scan and construct candidates with the existing ownership, archived-state,
   path, lock and metadata checks. Keep unsafe candidates visible as Blocked.
2. Probe `codex --version` in preview and Diagnostics, with `CODEX_HOME` bound
   to the detected state root. A three-second deadline bounds launchers; the
   direct child is killed on cancellation. Missing/unexecutable binaries, non-zero probes
   and versions other than `codex-cli 0.152.0` produce explicit diagnostics.
3. Always reject cleanup validation with the independent complete-recovery
   blocker. When CLI admission also fails, preserve both explanations.
4. Reject direct provider execution too. It cannot invoke native deletion,
   create backups or replace databases by bypassing the preview gate.

Before re-enabling mutation, prove a successful native delete and complete
recovery of both databases, transcript, index and attachments using controlled
samples on the target platform. Persist recovery locations before mutation;
do not infer recovery from version/help output or SQLite `quick_check` alone.

The shared executor also rejects a selection containing multiple operations
on the same physical database **before any operation in that selection**.
Canonical aliases and hard links are included in this check. Selecting only
one DB session is allowed by this generic gate, but does not bypass Codex's
independent recovery blocker. Coordinated DB batch execution needs a separate
provider contract; silently rebasing DB/WAL evidence after a delete would hide
external changes and is not permitted.

### Deferred self-implementation (§12.4)

The former prototype replaced only `state_5.sqlite` and did not prove a
complete multi-database/file consistency set, source-WAL replacement or
Windows atomic replacement. It is removed from the production provider.
Re-enablement needs dedicated safety/compatibility tests and a documented
recovery procedure first; CLI errors must never trigger this fallback.

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

Known bound: history/projection rows are not individually fingerprinted.
This is not a proof of complete multi-file recovery; production cleanup
therefore remains Blocked.

## Path safety (§16.1)

`validate_cleanup_unit` runs at plan build and rejects a unit unless:

- the `database` path in the `CleanupLocator::DbRow` is non-empty and
  free of reserved components (`paths::has_reserved_component`);
- the database canonicalizes to a path inside the installation's data
  roots (no symlink escapes);
- the `record_id` is non-empty and matches the snapshot's session id
  (no fabricated identifiers);
- a fresh scan still produces the same unit, locator and action with a
  cleanable risk level; the linked rollout and all previewed evidence must
  remain unchanged. Missing or reactivated sessions are skipped.

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
provider validation error and the user is asked to retry.

## Audit log (§16.3)

A synced `Pending` intent precedes each attempted mutation, followed by its
result (`agenttidy.audit.v1`) at
`$HOME/.agenttidy/operations.jsonl`: plan id, unit id, provider, the
`db-row` locator (database path + record id), action
`provider-operation`, outcome, estimated/actual reclaimed bytes and
the backup paths used for recovery. No transcript bodies are read into
plans or audit events; native stdout is not logged. No SQL statements are
logged in audit events.