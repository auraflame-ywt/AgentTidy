# Session Transcripts (`file-set`)

Status: **cleanup enabled (Phase 7 beta)** for the Claude Code provider
(`claude-code:cli` and the `claude-code:desktop` Desktop transcript mirrors).
Codex and WorkBuddy are not enabled — see
[`workspace-cleanup.md`](./workspace-cleanup.md) and
[`../providers/codex.md`](../providers/codex.md) for their gates.

This document covers the `file-set` resource type: one agent session's
transcript plus its co-located side files, treated as one atomic cleanup
unit (`Start.md` §9.5). The general cleanup execution contract — risk
vocabulary, §12.2 revalidation triggers, two confirmations, audit log —
lives in [`workspace-cleanup.md`](./workspace-cleanup.md) and applies
unchanged.

## What forms a unit

One unit per **recognized, exclusively-owned session resource**:

| Member | Path | Role |
| --- | --- | --- |
| Transcript | `<root>/projects/<slug>/<session-id>.jsonl` | Primary member; carries the §12.2 fingerprint (identity, size, mtime) |
| Side directory | `<root>/projects/<slug>/<session-id>/` | Optional; tool results, subagent transcripts (whole directory) |

`<root>` is the installation data root (`~/.claude` for the CLI; the
verified `.claude` mirror root under `local-agent-mode-sessions/` for the
Windows Desktop installation). The exact file set is what the read-only
scan already measured and displayed — the cleanup unit never adds or
discovers paths at cleanup time.

**Never forms a unit** (stays report-only, §3.2 "Never Act on What We Do
Not Understand" and §7.3):

- transcripts whose first-line `sessionId` is missing or does not match
  the filename (unrecognized ⇒ `Unknown` resource, `Ownership::Unknown`);
- `Unknown`-kind resources and anything under `include_unknown`;
- nested subagent files (measured inside the parent's side directory, but
  never a separate unit);
- anything outside `projects/<slug>/` — VM images, credentials, audit
  logs and user output directories of the Desktop installation are out of
  scope entirely (`Start.md` §2.3).

## Operation and recovery

- Action is **`trash` only**: members are moved to the OS trash
  (Finder Trash / Recycle Bin), one member path at a time
  (`move_paths_to_trash`). Recovery is performed **by the system**, from
  its trash UI — AgentTidy shows no in-app Undo (`Start.md` §16.4).
- The multi-path move is **not globally atomic**: if one member fails
  (e.g. a locked file on Windows), members that already moved are in the
  OS trash and are recoverable; the item is reported `failed` with the
  moved count and the verbatim platform error (§12.3 — partial failure
  is reported honestly, never as success). Re-running is safe: already
  gone members count as idempotent no-ops, not fresh success.
- No database rows are touched: Claude Code sessions are file-owned.
  (Contrast Codex, where transcripts and `state_5.sqlite.threads` rows
  must move together — that `provider-operation` gets its own doc.)

## Pre-conditions (declared at plan time, checked before execute)

The provider declares these typed §12.2 triggers on every unit; the
revalidator reports the first failing one per item:

| Trigger | Meaning for this resource type |
| --- | --- |
| `path-exists` | Transcript **and** side directory (when present) still exist |
| `path-unchanged` | Members canonicalize to the scanned locations (§16.1) |
| `identity-unchanged` | Transcript file identity (volume + index / dev + ino) unchanged |
| `size-unchanged` | Transcript size unchanged |
| `mtime-unchanged` | Transcript mtime unchanged — doubles as the session-re-activation trigger (an active session writes its transcript) |
| `no-agent-running` | No Claude process detected at execute time (§16.2) |
| `session-inactive` | Owning session lifecycle is still `Inactive`/`Archived` |

Known bound (documented limitation of the beta): drift *inside* the side
directory is not individually fingerprinted; it is covered by the
`no-agent-running` + `mtime-unchanged` pair and the whole directory is
moved as-is.

## Path safety (§16.1)

`validate_cleanup_unit` runs at plan build and rejects a unit unless:

- every member path is non-empty and free of reserved components
  (Windows device names, `\\?\` quirks handled by `paths::has_reserved_component`);
- every member **canonicalizes inside the installation data roots** —
  a member symlinked out of the root canonicalizes outside and is
  rejected (no symlink escapes);
- all members share one project directory (transcript + side directory);
- the shared directory sits at `<root>/projects/<slug>/` — the verified
  layout shared by CLI transcripts and Desktop mirrors.

Deletion targets come from the scan snapshot only; user-typed paths are
never accepted (§16.1 red line).

## Concurrency (§12.1, §16.2)

Execution holds the cross-process operation guard
(`$HOME/.agenttidy/operations.lock`) for the whole batch: a second GUI
window or CLI instance cannot execute concurrently. A live Claude
process blocks execution at both policy (`agent-running`) and
revalidation (`no-agent-running`) time. A file locked by another
process is classified by the platform error code
(`ERROR_SHARING_VIOLATION` / `ERROR_LOCK_VIOLATION`, raw and HRESULT
forms) into `TrashOutcome::Locked`: when it is the only failure and
nothing has moved yet, the item is reported `skipped` with a
close-the-holder-and-retry hint (transient condition); a lock that
arrives after other members already moved keeps the honest `failed`
(§12.3 — a partially-moved file set is never a clean skip).

## Audit log (§16.3)

A synced `Pending` intent precedes each mutation, followed by its final
`agenttidy.audit.v1` result at
`$HOME/.agenttidy/operations.jsonl`: plan id, unit id, provider, the
`file-set` locator (all original member paths), action `trash`, outcome,
estimated/actual reclaimed bytes and skip/failure reasons. No message
bodies, tokens or file contents are ever logged.


## macOS implementation / acceptance (2026-10-20)

Native Trash uses NSFileManager rather than Finder AppleScript; no Finder
Automation grant is required by this path. Manual move-back is supported,
but the Finder “Put Back” menu is not guaranteed. A strict temporary-file /
nested-directory Trash-and-restore test passed on the local Mac, including
spaces, CJK and quotes. Restricted sandbox refusal is an explicit failure.

Trash moves report zero physical reclaimed bytes; preview sizes are estimates,
and AgentTidy never empties Trash. Execution uses an OS advisory lock (released
on process exit/crash), a pre-opened audit sink, synced Pending intent and final
result. An unavailable audit sink prevents mutation; result-write failure stops
later items. See [`../testing/macos-scenarios.md`](../testing/macos-scenarios.md).
