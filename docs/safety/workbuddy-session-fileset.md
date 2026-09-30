# WorkBuddy Session Transcripts (`file-set`)

Status: **cleanup enabled (Phase 7 beta)** for the WorkBuddy provider.
This doc covers WorkBuddy's `file-set` resource type — the equivalent
of the Claude Code transcript file set, with WorkBuddy's specific
sidecar pattern and database cross-check. The general cleanup
execution contract — risk vocabulary, §12.2 revalidation triggers,
two confirmations, audit log — lives in
[`workspace-cleanup.md`](./workspace-cleanup.md) and applies unchanged.
Claude Code's parallel doc is
[`session-transcript-fileset.md`](./session-transcript-fileset.md);
Codex's `provider-operation` doc is
[`provider-operation.md`](./provider-operation.md).

## What forms a unit

One unit per **recognized, exclusively-owned session resource** —
WorkBuddy transcripts live in a directory layout analogous to Claude
Code's (`<root>/projects/<slug>/<session-id>...`) but with a richer
sidecar set per session id:

| Member | Path | Role |
| --- | --- | --- |
| Transcript | `<root>/projects/<slug>/<session-id>.jsonl` | Primary member; carries the §12.2 fingerprint (identity, size, mtime) |
| Side directory | `<root>/projects/<slug>/<session-id>/` | Optional; tool results, subagent transcripts (whole directory) |
| Meta sidecar | `<root>/projects/<slug>/<session-id>.meta.json` | Optional; host connection metadata for the session |
| Rollback sidecar | `<root>/projects/<slug>/<session-id>.file-rollback.ndjson` | Optional; per-session file-level rollback log |

`<root>` is the WorkBuddy installation data root
(`~/.workbuddy/` on macOS, `%USERPROFILE%\.workbuddy\` on Windows).
Only sidecars that exist on disk are listed in the unit; the unit
never adds or discovers paths at cleanup time.

**Never forms a unit** (stays report-only, §3.2 + §7.3):

- transcripts whose first-line `sessionId` is missing or does not
  match the filename (unrecognized ⇒ `Unknown` resource,
  `Ownership::Unknown`);
- `Unknown`-kind resources and anything under `include_unknown`;
- nested subagent files (measured inside the parent's side directory,
  but never a separate unit);
- anything outside `projects/<slug>/` — `workbuddy.db`,
  `blobs/`, `file-history/`, `clipboard-images/`, `traces/`, `logs/`
  and `audit-log/` are out of scope entirely (`Start.md` §2.3);
- WorkBuddy's default workspace directory (`~/WorkBuddy/`) — it is
  user-mixed (automations can reference any cwd) and remains
  report-only until the automation-references check below passes.

## Database cross-check

A WorkBuddy session is *only* a candidate unit when its lifecycle
can be determined from the `sessions` table — without that signal
the provider reports `SessionLifecycle::Unknown` and the §11.2
fail-closed rule blocks the unit. The cross-check is:

| `sessions.deleted_at` | `sessions.status` | `sessions.updated_at` vs. now | SessionLifecycle |
| --- | --- | --- | --- |
| NOT NULL | any | any | `Archived { archived_at: updated_at }` |
| NULL | `archived` | ≥ 30 days | `Inactive` |
| NULL | `archived` | < 30 days | `Inactive` (conservative — WorkBuddy's archived flag is not a safe permission per red line #5) |
| NULL | not `archived` | < 30 days | `Inactive` |
| NULL | not `archived` | ≥ 30 days | `Inactive` |
| (DB unreadable) | — | — | `Unknown` — unit dropped (§11.2) |

`Archived` sessions are *eligible* for a unit but per red line #5 the
session's archive state is *not* by itself a permission to clean —
the §11.2 four-gate (`Archived + silent ≥ 30 days + exclusive +
recoverable`) still applies at policy time, and WorkBuddy's archived
sessions default to `ReviewRequired` (the GUI surfaces the user-
facing reason "session was archived but the user's recent activity in
the workspace may still reference it").

## Operation and recovery

- Action is **`trash` only**: members are moved to the OS trash
  (Finder Trash / Recycle Bin), one member path at a time
  (`move_paths_to_trash`). Recovery is performed **by the system**,
  from its trash UI — AgentTidy shows no in-app Undo
  (`Start.md` §16.4).
- The multi-path move is **not globally atomic**: if one member fails
  (e.g. a locked file on Windows — see "Concurrency"), members that
  already moved are in the OS trash and are recoverable; the item is
  reported `Failed` (or `Skipped` for a locked-only failure) with the
  moved count and the verbatim platform error (§12.3 — partial
  failure is reported honestly, never as success). Re-running is
  safe: already gone members count as idempotent no-ops, not fresh
  success.
- No `workbuddy.db` rows are touched: this is a file-only resource —
  the `sessions` row for the deleted thread remains in the database and
  must be cleaned separately (not currently exposed; the row's
  `deleted_at` is already non-null, the only purpose of a future
  `provider-operation` would be reclaiming the DB row bytes).

## Pre-conditions (declared at plan time, checked before execute)

The provider declares these typed §12.2 triggers on every unit; the
revalidator reports the first failing one per item:

| Trigger | Meaning for this resource type |
| --- | --- |
| `path-exists` | Transcript **and** every listed sidecar (when present) still exist |
| `path-unchanged` | Members canonicalize to the scanned locations (§16.1) |
| `identity-unchanged` | Transcript file identity (volume + index / dev + ino) unchanged |
| `size-unchanged` | Transcript size unchanged |
| `mtime-unchanged` | Transcript mtime unchanged — doubles as the session-re-activation trigger (an active session writes its transcript) |
| `no-agent-running` | No WorkBuddy process detected at execute time (§16.2) |
| `session-inactive` | Owning session lifecycle is still `Inactive`/`Archived` and the snapshot's lifecycle source is still valid |
| `schema-unchanged` | `workbuddy.db` `user_version` + `sessions` schema unchanged from the snapshot |

Known bound (documented limitation of the beta): drift *inside* the
side directory is not individually fingerprinted; it is covered by
the `no-agent-running` + `mtime-unchanged` pair and the whole
directory is moved as-is.

## Workspace check (`~/WorkBuddy/`)

The `~/WorkBuddy/` default workspace directory is *only* a unit when
no automation references its path. The provider's `scan()` reads
`automations.cwds` (the JSON array of cwds each automation operates
on) and exposes the result on the workspace resource as
`metadata["automation_cwd_refs"]`. The policy rule is:

- absent metadata → Blocked (reason: "WorkBuddy automation
  references are not yet verified");
- metadata present and contains the workspace path → Blocked
  (reason: "WorkBuddy automation references this cwd");
- metadata present and empty (or absent path) → eligible, subject to
  the same `file-tree` eligibility checks as any other workspace.

## Path safety (§16.1)

`validate_cleanup_unit` runs at plan build and rejects a unit unless:

- every member path is non-empty and free of reserved components
  (Windows device names, `\\?\` quirks handled by
  `paths::has_reserved_component`);
- every member **canonicalizes inside the installation data roots** —
  a member symlinked out of the root canonicalizes outside and is
  rejected (no symlink escapes);
- all members share one project directory (transcript + side
  directory + sidecars);
- the shared directory sits at `<root>/projects/<slug>/` — the
  verified WorkBuddy session layout;
- for the cross-check path: the `sessions` row referenced by the
  transcript's session id still exists in `workbuddy.db` and its
  `deleted_at IS NOT NULL` flag is unchanged from the snapshot.

Deletion targets come from the scan snapshot only; user-typed paths
are never accepted (§16.1 red line).

## Concurrency (§12.1, §16.2)

Execution holds the cross-process operation guard
(`$HOME/.agenttidy/operations.lock`) for the whole batch: a second
GUI window or CLI instance cannot execute concurrently. A live
WorkBuddy process blocks execution at both policy (`agent-running`)
and revalidation (`no-agent-running`) time. A file locked by another
process on Windows surfaces as `PathTrashOutcome::Locked` and the
item is reported `Skipped` with reason "file locked by another
process" (see `trash.rs` for the platform error-code mapping).

## Audit log (§16.3)

One `agenttidy.audit.v1` row per item at
`$HOME/.agenttidy/operations.jsonl`: plan id, unit id, provider, the
`file-set` locator (all original member paths), action `trash`,
outcome, estimated/actual reclaimed bytes and skip/failure reasons.
No message bodies, tokens or file contents are ever logged.