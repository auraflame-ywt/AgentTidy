# Default Workspace Cleanup Safety Rules

This document defines the cleanup gate for default agent workspaces and the
general execution contract shared by every cleanup-capable resource type. The
rules implement the safety principles in `Start.md` §16 and are enforced by
the Phase 6 planner (`crates/application`) and executor.

**Current status (Phase 6):** the framework, policy, revalidation protocol,
two-confirmation flow, audit log, and GUI Review surface are live — but no
provider emits cleanup units yet, so nothing on disk is cleanable. Workspace
cleanup specifically stays deferred: `Start.md` §2.3 keeps files already in
the user's project out of default cleanup, and `~/Documents/Codex/` /
`~/WorkBuddy/` are user-mixed roots. Any future enablement of a new resource
type requires a safety doc here first (see `docs/safety/README.md`).

## Risk vocabulary

Every plan item carries one of the three `Start.md` §7 risk levels — the
former binary eligible/blocked model is replaced:

| Risk | GUI badge (English) | Reference (Chinese) | Default selection (§6.3) |
| --- | --- | --- | --- |
| `low-risk` | Recommended (green) | 建议清理 | Checked, user may uncheck |
| `review-required` | Caution (amber) | 谨慎清理 | Unchecked |
| `blocked` | Off-limits (gray) | 很干净 | Non-selectable |

`low-risk` requires every §7.1 condition simultaneously: known resource
kind, known provider version and schema, an atomic cleanup unit, exclusive
ownership, no unknown dependencies, a non-active session, recoverable via
OS Trash, and re-checkable preconditions. Archive alone (red line #5) or
age alone never promotes a unit to `low-risk`.

## Eligibility

A workspace may be proposed only when all of the following facts are proven
at plan time and revalidated immediately before execution:

- It is under a provider's documented default-generated workspace root, not a
  user-selected cwd or an arbitrary project directory.
- Its full canonical path has an exact one-to-one relationship with one known
  session. Slug decoding or path-name guesses are never sufficient proof.
- No other session, project record, automation, or workspace registry entry
  references the same path or a descendant of it.
- The directory tree contains no `.git` directory or file at any depth.
- The provider scan has not reported unreadable, linked, unknown, or changed
  entries within the candidate workspace.

Any missing, stale, shared, unknown, or conflicting evidence blocks cleanup.

## Revalidation triggers

Between the two confirmations and again inside the executor, every selected
unit is revalidated (`Start.md` §12.2). A unit is `skipped` — never executed —
when **any** of these changed since plan time:

1. **Path exists** — the recorded path no longer resolves.
2. **Path unchanged** — `safe_canonicalize` resolves elsewhere (link swap,
   Windows verbatim/UNC re-spelling).
3. **Identity unchanged** — file identity (Windows volume + file index,
   Unix dev + inode) differs: the original was replaced.
4. **Size unchanged** — logical size differs.
5. **Mtime unchanged** — modification time moved more than tolerance.
6. **No agent running** — a fresh `inspect()` reports the agent process
   present (§16.2: concurrent execution is unsafe).
7. **Session inactive** — the owning session returned to `Active`, or its
   lifecycle is `Unknown`.
8. **Session still exclusive** — the workspace no longer maps 1:1 to
   exactly one session cwd (§12.2 "dependencies changed / no longer
   exclusive").
9. **Schema unchanged** — provider schema versions / journal modes differ
   from the plan-time inspection.
10. **Writer lock absent** — e.g. Codex `thread-writer-locks/` is no longer
    empty.
11. **Filesystem eligible** — the four workspace-safety booleans
    (`filesystem_eligible`, `has_git_marker`, `has_unsafe_entry`,
    `has_unreadable_entry`) changed.
12. **No symlink outside root** — a link inside the unit now points outside
    it (§16.1 link boundary).

Each trigger is a typed `CleanupPreconditionKind`; the executor reports
*which one* fired so the GUI can show a per-row "changed — rescan" badge.
The plan also expires after its `expires_at` TTL — an expired plan requires
a fresh scan, not a re-click (§12.2: 计划已过期，请重新扫描).

## User Confirmation

Cleanup requires two separate affirmative confirmations:

1. The user reviews a non-mutating CleanupPlan that names each unit, owning
   session, measured size, risk level, and every failed-or-passed safety gate
   ("Review Selected" in the GUI, which triggers revalidation).
2. Immediately before execution, after revalidation, the user confirms the
   same selection again. A changed candidate invalidates the first
   confirmation and requires a new plan review. The GUI additionally
   requires typing `CONFIRM` — no keyboard shortcut or default focus
   reaches the execute button.

Both confirmations must prominently state that the directory can contain
agent-generated files the user may still need, that recovery relies on the OS
Trash, and that AgentTidy cannot guarantee semantic recovery of external
references.

The executor enforces the confirmations technically via a plan
`fingerprint` (SHA-256 over the plan tuple): `cleanup_execute` refuses any
plan whose fingerprint does not match what the backend re-derives.

## Execution Boundary

Two action types exist and must never be confused (`Start.md` §3.4, §12.4):

- `action: trash` — file-shaped units move to the OS Trash (Windows Recycle
  Bin / macOS Finder) as **one whole**; AgentTidy never partially deletes
  contents. Database rows are never labeled "Move to Trash".
- `action: provider-operation` — a provider-coordinated operation (backup
  then vacuum, etc.). Not constructed in Phase 6; requires its own safety
  doc before enablement.

Outcomes are recorded per item (`Start.md` §12.3): `removed` / `skipped` /
`failed`. A path already gone at execute time counts as `skipped`, never as
a new success (idempotent reporting). Partial failure never corrupts the
report — each remaining unit still executes and records independently. No
cross-database global transaction is promised in v0.1.

**Operation guard (§12.1)** is acquired before revalidation and held for
execution/audit. Every selected item is re-scanned and its unit, policy,
provider validation, canonical paths and recursive member fingerprints are
checked immediately before its mutation. Independent IPC requests discover
installations afresh; there is no preview-instance cache. Full plan payloads
and expiry are fingerprinted. Selected stale/blocked items are audited as
`skipped`; each item also checks TTL after its revalidation.

File-set membership, nested member metadata and inspection evidence are
stored in the preview. These are metadata checks, not content hashes, and
cannot prevent an external writer starting after the final check. Native
integration and recovery validation remain release gates.

## Audit log

Every executed item appends one line to the per-user audit log
(`Start.md` §16.3):

- Path: `$HOME/.agenttidy/operations.jsonl` (`%USERPROFILE%\.agenttidy\`
  on Windows), append-only, `fsync`ed per line, no rotation in v0.1
  (user-managed truncation).
- Fields: `schema_version` (`agenttidy.audit.v1`), `platform`,
  `timestamp_ms`, `plan_id`, `unit_id`, `provider`, `original_locator`
  (tagged: file / dir / file-set / db-row), `action`, `outcome`,
  `reclaimed_bytes`, `reason`.
- Hard constraint: the log never contains transcript bodies, credentials,
  tokens, or file contents — only paths and metadata. It must be safe to
  ship in a diagnostics bundle without redaction.

Eligible workspaces are moved to the OS Trash as one directory; AgentTidy
never partially deletes their contents. User-selected projects,
repositories, shared workspaces, and all non-default locations stay
Blocked.

### Mac execution hardening (2026-10-20)

- Provider validation/evidence failure becomes a Blocked row with its reason;
  it does not erase healthy rows or other providers from the preview. Blocked
  bytes are excluded from `total_reclaimable_bytes`.
- The persistent operation-lock file is protected by an OS advisory lock.
  Process exit releases ownership; file age never grants takeover. Do not
  unlink a live lock file. Unknown legacy stamps require manual inspection
  after older clients have stopped, not automatic removal.
- The audit sink opens before mutation. A synced Pending intent precedes each
  operation, followed by a final result; skipped selections record a final
  event without intent. A missing final result means uncertain execution,
  never proof of rollback or success. Truncated crash lines are separated
  before appending new events. Logging failure stops subsequent mutations.
- Actual reclaimed bytes are zero for a Trash move. The selected/preview size
  is an estimate; files in Trash still occupy disk space. No automatic purge.

Local evidence and remaining acceptance items:
[`../testing/macos-scenarios.md`](../testing/macos-scenarios.md).
