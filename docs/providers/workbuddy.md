# WorkBuddy Provider Investigation

> Status: **In progress** — macOS sample collected 2026-09-19 (WorkBuddy 5.5.6, build 5f96929).
>
> Windows paths not yet verified. Filled from real samples, not guesses.

## Verified version matrix

| Platform | App version | Sample date | Notes |
|---|---|---|---|
| macOS (arm64, Electron app) | 5.5.6 (`last-launch.json`: version/build/timestamp) | 2026-09-19 | sessions DB covers 2026-06 → 2026-09 |

## Installation & data locations (macOS)

Root: `~/.workbuddy/` (4.4 GB total in sample). App installed at `/Applications/WorkBuddy.app`; bundled CLI `workbuddy` elsewhere. **Additional scan locations beyond the state root** (see "Default workspace directories" below): `~/WorkBuddy/` and per-workspace nested `.workbuddy/` dirs.

| Path | Role | Size (sample) | Cleanup class |
|---|---|---|---|
| `workbuddy.db` (+wal/shm, Drizzle migrations) | **Session index DB** | 2 MB | Index — never delete rows |
| `.workbuddy-sqlite-migrations/*.sql` | Applied migration SQL files | — | Never clean |
| `projects/<cwd-slug>/<uuid>.jsonl` (+ `<uuid>/tool-results/`) | Session transcripts (JSONL) + per-session tool results | 270 MB | Per-session |
| `logs/YYYY-MM-DD/*.log`, `logs/<hash>.log` | Per-day app logs | 657 MB | Log class |
| `traces/<id>/` | Per-run traces | 481 MB | Log/trace class |
| `binaries/node`, `binaries/python` | Bundled runtimes (1.9 GB!) | 1.9 GB | Program files — never clean |
| `app/` | Electron app data (cache, session, crashpad, `sessions.json`) | 420 MB | App state |
| `plugins/`, `connectors-marketplace/`, `appearance-resources/`, `buddy-skill-store/` | Installed extensions | ~300 MB | Program-ish files |
| `blobs/` (sharded `00`..`ff`) | Content-addressed blobs | 102 MB | Shared |
| `shell-snapshots/` (812 files) | Shell env snapshots | 111 MB | Cache class |
| `clipboard-images/` (115) | Pasted images | 18 MB | Per-use |
| `file-history/<uuid>/` | File snapshots (`<hash>@v<n>`) | 20 MB | Shared / review |
| `audit-log/YYYY-MM-DD.jsonl` | Append-only audit trail | — | Never clean (compliance) |
| `changes-index/`, `changes-detail/`, `file-tree-manifests/`, `artifact-index/` | Derived indices | ~50 MB | Derivable cache |
| `edge-sync-mapping-v{1..4}.db` (+wal/shm) | Edge sync mappings — **four generations coexist**, v1–v3 stale | ~3 MB | Old generations = stale |
| `db-backups/`, `automation-backups/` | Backups | — | Backup class |
| `workspace/`, `storage/`, `security/`, `credentials/`, `memory/` | Identity / user data | — | Never clean |
| `MEMORY.md`, `SOUL.md`, `IDENTITY.md`, `USER.md`, `BOOTSTRAP.md` | Persona/identity files | — | Never clean |
| `models.json` | User model configs — **contains API keys in plaintext** | — | Never clean / never log |
| `usage-log.json`, `user-state.json`, `settings.json`, `last-launch.json` | App state | — | Never clean |

## Version discovery mechanism

- `~/.workbuddy/last-launch.json` → `{"version":"5.5.6","build":"5f96929…","timestamp":…}` (authoritative, updated each launch).
- DB migrations: `__workbuddy_drizzle_migrations` + `migration_meta` tables.

## Session identity & storage structure

- **Session ID**: UUIDv4. Two-layer storage:
  1. `workbuddy.db.sessions` row (id, cwd, user_id, title, status, created_at, updated_at, deleted_at, source_mode (`working`/`craft`/`design`), model, project_id, buddy_snapshot_id, …) — 305 rows in sample.
  2. Transcript on disk: `projects/<cwd-slug>/<uuid>.jsonl` (slug scheme: `/` → `-`, **all other characters preserved verbatim** — spaces, CJK, dots survive; see "User-chosen working directories" below), plus optional sidecar files:
     - `<uuid>/` dir (`tool-results/`)
     - `<uuid>.meta.json` (e.g. host connection metadata)
     - `<uuid>.file-rollback.ndjson` (file-rollback journal)
- Transcript line types (sample): `message` (role user/assistant), `function_call`, `function_call_result`, `reasoning`, `file-history-snapshot`. Lines carry `id`, `timestamp` (epoch ms), `sessionId`, `cwd`, `providerData`.
- `session_usage(session_id, used, size, updated_at, credit_json)` — per-session size accounting already exists in-app.
- `workspaces(path PRIMARY KEY, last_opened_at)` — known working directories (14 in sample).
- `buddy_snapshots` — agent persona snapshots referenced by `sessions.buddy_snapshot_id`.
- `automations`, `automation_runs`, `automation_delivery_outbox`, `automation_runtime_state` — scheduled automations (14 in sample), referencing `cwds` (JSON array of paths).

### User-chosen working directories (critical)

A WorkBuddy conversation's cwd is **not** restricted to a project checkout: the user picks any directory at session creation (default scratch dirs like `~/WorkBuddy/2026-08-14-09-00-52`, but also arbitrary named dirs — sample includes `/Users/lxy/WorkBuddy/Telegram 方向竞品调研` and `/Users/lxy/WorkBuddy/个人社媒`, i.e. **spaces and CJK in both cwd and thus on-disk slug dirs**).

Implications:

- **Slug ↔ cwd is a bijection** (only `/` → `-`; no escaping/hashing). Decoding a slug back to a cwd is exact — including case, spaces, Unicode. Never normalize NFC or strip characters when mapping; store paths as-is.
- **`-` is ambiguous in reverse mapping** (`/a/b` and `/a-b` produce different slugs, but a cwd *containing* `/`-adjacent hyphens still round-trips since only separators are replaced — decode requires trying the DB `sessions.cwd` as source of truth instead of blind slug inversion when in doubt).
- The cwd may point at **user content** (e.g. `~/WorkBuddy/半自动社媒` holds the user's own files). The transcript lives under `~/.workbuddy/projects/…` — separate from the workspace itself; we only ever touch the transcript side.
- `automations.cwds` and `workspaces` reference these arbitrary cwds too — the "is this directory still in use?" check must consult both tables, not just sessions.
- Windows (unverified): cwd may be a different drive (`D:\…`); slug mapping on `\` separators needs a real Windows sample before freezing the algorithm.

### Default workspace directories & nested agent state (critical for scanning)

WorkBuddy's default conversation workspace root is **`~/WorkBuddy/`** (note: case-insensitive APFS tolerates `~/Workbuddy/`; observed both spellings — scan must compare case-insensitively on macOS and treat them as the same root):

1. **Default scratch workspaces**: `~/WorkBuddy/YYYY-MM-DD-HH-MM-SS/` (auto-named, one per new conversation) and `automation-<ts>/` (automation workspaces). Sample: 84+ timestamp dirs. These accumulate forever; they contain agent-written files the user may or may not still want → **Blocked** for cleanup, valuable for space attribution.
2. **Named workspaces**: user-named dirs directly under `~/WorkBuddy/` (e.g. `个人社媒/`, `WABA/`, `Claw/`) — user content, Blocked.
3. **Nested `.workbuddy/` state dirs inside workspaces**: `~/WorkBuddy/.workbuddy/memory/` (daily memory notes `YYYY-MM-DD.md`) and `.workbuddy/automations/<id>/memory.md` — WorkBuddy writes agent state **inside the workspace** itself. Any "clean a workspace dir" reasoning must account for hidden agent state; conversely, deleting session data in `~/.workbuddy` does not remove these.
4. Loose files at workspace root (sample: `x_posts_record.json` + 4 `.bak` variants, generated scripts/HTML) — agent-produced user artifacts, Blocked.

**Scan locations for WorkBuddy** (deduplicated, case-insensitive on macOS):

| Location | Content | Role in app |
|---|---|---|
| `~/.workbuddy/` | transcripts, DBs, caches (this doc) | State root — primary scan target |
| `~/WorkBuddy/` (any case) | default scratch workspaces + named workspaces | Workspace root — report-only (Blocked) |
| `~/WorkBuddy/.workbuddy/` | workspace-scoped agent memory/automation notes | Hidden agent state inside user-visible tree |
| `/Applications/WorkBuddy.app` | app bundle | Detection only (installed? version?) |

Correlation: `workspaces` table + `sessions.cwd` + `automations.cwds` tell which workspace dirs are referenced; **81 of 305 sampled sessions have cwd absent from `workspaces`** — workspace registry is NOT authoritative for "is a directory still used"; consult sessions/automations too. Space attribution for `~/WorkBuddy/*` goes through the sessions' `cwd` values, same as any user-chosen directory.

## Archive / lifecycle semantics

- **Soft delete**: `sessions.deleted_at` (268 of 305 rows deleted in sample — deleted rows may keep transcripts on disk → orphaned transcript files are a real cleanup target).
- `status` values: `completed` (243), `archived` (59), `error` (2), `terminated` (1) — archive is an in-DB status, transcripts stay in place.
- **Archive ≠ permission** (red line #5) — archived status alone must not lower risk class.

## Project mapping

- `sessions.cwd` (absolute path) + filesystem slug dirs; `projects/<cwd-slug>/` mirrors cwd one-to-one.
- `workspaces` table = "known dirs" registry; `workspace-display-names.json` gives display names.
- No separate projects table (unlike Codex); project = cwd.

## Resource dependency graph

```
~/.workbuddy
├── workbuddy.db
│   ├── sessions (id, cwd, deleted_at, buddy_snapshot_id …)
│   ├── workspaces (path)
│   ├── session_usage (session size)
│   └── automations → cwds[] (workspace paths)
├── projects/<slug>/<uuid>.jsonl          (transcript; slug↔cwd)
│   ├── <uuid>/tool-results/             (owned by session <uuid>)
│   ├── <uuid>.meta.json                 (session-scoped sidecar)
│   └── <uuid>.file-rollback.ndjson      (session-scoped sidecar)
├── blobs/<shard>/                        (content-addressed, shared)
├── file-history/<uuid>/<hash>@v<n>/      (file snapshots)
├── clipboard-images/                    (pasted images)
├── traces/<run-id>/                      (per-run traces)
└── logs/YYYY-MM-DD/*.log
```

Deleting a transcript while leaving `sessions` row (or vice versa) creates dangling references; `session_usage` rows orphan too. Automations may reference a cwd whose project dir is being considered for cleanup — check `automations.cwds` before touching a project dir.

## Caches, logs, checkpoints

- `logs/` (657 MB) and `traces/` (481 MB) are the big recoverable space, keyed by date/id — but both may be needed for support/audit; treat as Review, not auto-clean.
- `shell-snapshots/` (812 files, 111 MB) — cache class, safe-ish.
- `changes-*`, `artifact-index/`, `file-tree-manifests/` — derivable indices; regenerable in principle (verify before enabling cleanup).
- Old `edge-sync-mapping-v1..v3.db` generations — superseded by v4; stale-generation cleanup is a clear candidate unit.

## Databases & schema

- `workbuddy.db` (Drizzle ORM, WAL). Tables: `sessions`, `workspaces`, `session_usage`, `buddy_snapshots`, `automations`, `automation_runs`, `automation_delivery_outbox`, `automation_runtime_state`, migration tables.
- **`sessions` table contract — frozen v1 (2026-09-30, Phase 7)**: columns `id, cwd, user_id, title, status, created_at, updated_at, deleted_at, source_mode, model, project_id, buddy_snapshot_id` (see `fixtures/workbuddy/db/sessions-schema.json` for the sanitized snapshot with types/nullability). AgentTidy reads only `id, deleted_at, status, updated_at` for lifecycle inference: `deleted_at IS NOT NULL` ⇒ `Archived`; recognized terminal status (`completed`/`archived`/`error`/`terminated`) ⇒ `Inactive`; anything else ⇒ `Unknown` (fail-closed, `docs/safety/workbuddy-session-fileset.md`). The `automations.cwds` JSON array is also read (contract v1) to expose `automation_cwd_refs` on the workspace resource.
- `edge-sync-mapping-v{1..4}.db` — separate sync DBs, generation-numbered, all present simultaneously with their own WAL files.

## Runtime write behavior

- WorkBuddy is an Electron desktop app that stays resident (SingletonLock in `app/`); `workbuddy.db-wal` actively written.
- Transcripts appended per turn; `automation` runs may write at scheduled times (14 automations in sample) — "app not running" is not guaranteed safe; check lock/process.

## Atomic cleanup units

- ✅ **Enabled (Phase 7 beta)**: transcript `projects/<slug>/<uuid>.jsonl` + `<uuid>/` side dir + `<uuid>.meta.json` + `<uuid>.file-rollback.ndjson` (full session file set) for sessions whose DB-derived lifecycle is `Inactive`/`Archived` (`deleted_at IS NOT NULL` ⇒ Archived; terminal status ⇒ Inactive). Action is `Trash` (file-owned); **no DB rows are mutated** — the `sessions` row stays and the plan-build cross-check (`validate_cleanup_unit`) re-reads it fail-closed. See `docs/safety/workbuddy-session-fileset.md`.
- ✅ Candidate: `edge-sync-mapping-v1..v3.db(+wal/shm)` superseded generations (needs app-not-running + v4 healthy check).
- ❌ Not units: `binaries/`, `app/`, `plugins/` (program files), `blobs/` (shared, content-addressed, no verified back-refs), `audit-log/`, `models.json`, persona files.

## Known risks

- **Plaintext API keys in `models.json`** — never display, log, or include in fixtures; fixture sanitization must scrub this file entirely.
- Deleted sessions (88% of sample) leave transcripts on disk — user-visible "deleted" in-app does not mean disk-freed; AgentTidy must not assume DB state == disk state.
- Session dirs live in the **same tree** as nothing else session-scoped; but `projects/<slug>/` interleaves many sessions — never delete a whole slug dir on one session's behalf.
- Automations reference cwds; a "stale" project dir may still be an automation target.
- `~/WorkBuddy/` scratch workspaces (user data!) live outside `~/.workbuddy` — out of scope for cleanup, never touch; but they are **in scope for detection and space reporting** (default-workspace scan, see above).
- **Nested `.workbuddy/` state inside `~/WorkBuddy/`** means "agent data" and "user data" interleave in the same visible tree; a scanner must descend into workspace dirs to find hidden agent state, while cleanup of the surrounding user files stays Blocked.
- **User-chosen cwds can be arbitrary user directories** (spaces, CJK, possibly the user's document folders). Path handling must be Unicode-correct end-to-end; slug dirs with CJK names must not be treated as anomalies. Reverse slug→cwd mapping is ambiguous in theory — prefer `sessions.cwd` from the DB as the source of truth; only fall back to slug decoding when the DB row is missing, and mark such sessions for extra scrutiny.

## Capability degradation rules

- `workbuddy.db` missing/corrupt ⇒ fall back to slug-dir transcript scan (degraded: no deleted_at/status info ⇒ all sessions Unknown → Blocked for cleanup, list-only). Slug decoding without DB cross-check is lossy-risky (user cwds may contain arbitrary characters) ⇒ extra Blocked.
- Transcript first line not a `message` with `sessionId` ⇒ Unknown → Blocked.
- `last-launch.json` version older than min-supported ⇒ warn, degrade to list-only.

## Installation & data locations (Windows)

**Verified**: Windows sample collected 2026-09-20 (WorkBuddy 5.5.6, build 5f96929).

Root: `%USERPROFILE%\.workbuddy\` (e.g. `C:\Users\18712\.workbuddy\`). App installed elsewhere; bundled CLI `workbuddy` in PATH. **Additional scan locations beyond the state root**: `%USERPROFILE%\WorkBuddy\` (default workspace root).

| Path | Role | Size (sample) | Cleanup class |
|---|---|---|---|
| `workbuddy.db` (+wal/shm, Drizzle migrations) | **Session index DB** | 0.18 MB | Index — never delete rows |
| `.workbuddy-sqlite-migrations/*.sql` | Applied migration SQL files | — | Never clean |
| `projects/<cwd-slug>/<uuid>.jsonl` (+ `<uuid>/tool-results/`) | Session transcripts (JSONL) + per-session tool results | 45.73 MB / 44 files | Per-session |
| `logs/YYYY-MM-DD/*.log`, `logs/<hash>.log` | Per-day app logs | 108.01 MB / 143 files | Log class |
| `traces/<id>/` | Per-run traces | 0.78 MB | Log/trace class |
| `binaries/node`, `binaries/python` | Bundled runtimes | 712.68 MB | Program files — never clean |
| `app/` | Electron app data (cache, session, crashpad, `sessions.json`) | 63.09 MB | App state |
| `plugins/`, `connectors-marketplace/`, `appearance-resources/`, `buddy-skill-store/` | Installed extensions | — | Program-ish files |
| `blobs/` (sharded `00`..`ff`) | Content-addressed blobs | 14.71 MB | Shared |
| `shell-snapshots/` | Shell env snapshots | 5.83 MB | Cache class |
| `clipboard-images/` | Pasted images | 4.85 MB | Per-use |
| `file-history/<uuid>/` | File snapshots (`<hash>@v<n>`) | 3.08 MB | Shared / review |
| `audit-log/YYYY-MM-DD.jsonl` | Append-only audit trail | — | Never clean (compliance) |
| `changes-index/`, `changes-detail/`, `file-tree-manifests/`, `artifact-index/` | Derived indices | — | Derivable cache |
| `edge-sync-mapping-v{1..4}.db` (+wal/shm) | Edge sync mappings — **four generations coexist**, v1–v3 stale | ~0 MB | Old generations = stale |
| `db-backups/`, `automation-backups/` | Backups | — | Backup class |
| `workspace/`, `storage/`, `security/`, `credentials/`, `memory/` | Identity / user data | — | Never clean |
| `MEMORY.md`, `SOUL.md`, `IDENTITY.md`, `USER.md`, `BOOTSTRAP.md` | Persona/identity files | — | Never clean |
| `models.json` | User model configs — **contains API keys in plaintext** | — | Never clean / never log |
| `usage-log.json`, `user-state.json`, `settings.json`, `last-launch.json` | App state | — | Never clean |

**Windows-specific notes**:
- Path separator: `\` (backslash) instead of `/` (forward slash) on macOS.
- Slug mapping: `/` → `-` (same as macOS), but Windows paths use `\` as separator. Example: `C:\Users\18712\WorkBuddy\2026-06-25-22-33-03` → `c-Users-18712-WorkBuddy-2026-06-25-22-33-03`.
- Cross-drive cwds: possible (e.g. `D:\Projects\...`), but not observed in sample.
- Default workspace root: `%USERPROFILE%\WorkBuddy\` (confirmed).
- Case sensitivity: Windows is case-insensitive; compare paths case-insensitively.

**Default workspace directories & nested agent state (Windows)**:

| Location | Content | Role in app |
|---|---|---|
| `%USERPROFILE%\.workbuddy\` | transcripts, DBs, caches (this doc) | State root — primary scan target |
| `%USERPROFILE%\WorkBuddy\` | default scratch workspaces + named workspaces | Workspace root — report-only (Blocked) |
| `%USERPROFILE%\WorkBuddy\.workbuddy\` | workspace-scoped agent memory/automation notes | Hidden agent state inside user-visible tree |

**Scan locations for WorkBuddy on Windows** (deduplicated, case-insensitive):

| Location | Content | Role in app |
|---|---|---|
| `%USERPROFILE%\.workbuddy\` | transcripts, DBs, caches (this doc) | State root — primary scan target |
| `%USERPROFILE%\WorkBuddy\` | default scratch workspaces + named workspaces | Workspace root — report-only (Blocked) |
| `%USERPROFILE%\WorkBuddy\.workbuddy\` | workspace-scoped agent memory/automation notes | Hidden agent state inside user-visible tree |
| `%APPDATA%\WorkBuddy\` | Roaming AppData (settings, etc.) | App state |
| `%LOCALAPPDATA%\WorkBuddy\` | Local AppData (logs) | Logs |

## Open questions (Phase 0 continues)

- [x] Windows path (`%USERPROFILE%\.workbuddy` verified 2026-09-20; also `\`-separator slug mapping with cross-drive cwds on Windows, and the Windows default workspace root — `%USERPROFILE%\WorkBuddy\` verified).
- [ ] Does WorkBuddy have a native "empty trash" that also removes transcripts? (worth investigating before we clean orphans)
- [ ] `traces/` and `logs/` retention knobs in-app?
- [ ] Blob GC: are `blobs/` entries ever unreferenced? Needs write-path study.
- [ ] Confirm the full set of per-session sidecar filename patterns (`<uuid>.meta.json`, `<uuid>.file-rollback.ndjson` observed — are there more?).
- [ ] Is the default workspace root configurable in-app (registry/setting)? If so, discovery must read the config, not assume `%USERPROFILE%\WorkBuddy\`.
