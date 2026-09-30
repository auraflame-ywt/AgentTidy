# Codex Provider Investigation

> Status: **In progress** — macOS sample collected 2026-09-19 (codex-cli 0.152.0; sample sessions written by 0.133.0 – 0.152.0 line).
>
> Windows paths not yet verified. Filled from real samples, not guesses.

## Verified version matrix

| Platform | CLI version | Sample date | Notes |
|---|---|---|---|
| macOS (arm64, Darwin 25.6.0) | 0.152.0 | 2026-09-19 | `codex --version`; `version.json` records `latest_version` 0.153.4 |
| macOS | 0.133.0 – 0.145.0-alpha | via `threads.cli_version` / `session_meta.cli_version` | schema stable across the sampled range |

## Detected installations

Codex ships as both a CLI (`codex-cli`) and a Desktop app. **They share the same state root** — both write to `~/.codex/` (or `%USERPROFILE%\.codex\` on Windows), appending to the same `state_5.sqlite.threads`, the same JSONL rollouts, and the same `logs_2.sqlite`. There is no separate Desktop data root.

Therefore v0.1 exposes Codex as **one** `codex` installation whose `data_roots` are:

- The state root (`~/.codex/` / `%USERPROFILE%\.codex\`) — authoritative for sessions.
- The default Desktop workspace root (`~/Documents/Codex/` / `%USERPROFILE%\Documents\Codex\`) — created only by Desktop when the user starts a conversation without picking a folder; report-only (Blocked).

Per-session origin (`threads.thread_source`, `threads.originator`, JSONL `session_meta.payload.originator`) is what tells CLI vs Desktop sessions apart inside the Provider; no separate `codex:cli` / `codex:desktop` `AgentInstallation` is needed because there is no separate storage to dedupe.

## Installation & data locations (macOS)

Root: `~/.codex/`. Unlike Claude Code, Codex keeps **both** JSONL rollouts and several SQLite databases.

| Path | Role | Size (sample) | Cleanup class |
|---|---|---|---|
| `sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` | Session rollouts (append-only JSONL) | 394 MB / 91 files | Per-session |
| `archived_sessions/rollout-*.jsonl` | User-archived rollouts (flat) | 6.5 MB / 5 files | Per-session (archived) |
| `state_5.sqlite` (+ `-wal`/`-shm`) | **Thread index**: `threads`, `projects`, `thread_attachments`, … | 8.1 MB | Index — never delete rows directly |
| `thread_history_1.sqlite` (+wal) | Projection of rollout content (`thread_items`, `thread_turns`) | 128 MB | Index |
| `logs_2.sqlite` (+wal) | Append-only runtime logs (`logs` table) | 145 MB | Log cache class |
| `queue_1.sqlite`, `memories_1.sqlite`, `goals_1.sqlite` | Queue / memory / goals features | ≤1 MB each | Feature state |
| `session_index.jsonl` | UI thread list: `{id, thread_name, updated_at}` | 34 lines | Derivable index |
| `attachments/<uuid>/` | Per-thread attachment blobs | — | Per-thread |
| `cache/`, `.tmp/`, `shell_snapshots/`, `thread-writer-locks/`, `ipc/` | Caches / locks | — | Cache class |
| `config.toml`, `auth.json`, `installation_id`, `version.json`, `hooks.json`, `rules/` | Config / credentials | — | Never clean |
| `..codex-global-state.json.tmp-*` (14 files) | **Orphaned temp write leftovers** in `~/.codex/` root | ~1 MB | Garbage (see risks) |
| `models_cache.json`, `cc-switch-model-catalog.json` | Model catalog caches | — | Cache |
| `archived_sessions/` sibling dirs (`agents/`, `automations/`, `browser/`, `computer-use/`, `generated_images/`, `goals_1.sqlite`…) | Feature data | — | Not yet classified |

### Default workspace directories (outside `~/.codex`!)

Codex Desktop / Codex app creates **default per-day workspace dirs under `~/Documents/Codex/`** when the user starts a conversation without picking a folder:

```
~/Documents/Codex/YYYY-MM-DD/<chat-name>/        # e.g. 2026-09-10/new-chat/
├── work/                                       # agent working files
└── outputs/                                    # produced artifacts
```

- Verified in sample: 8 day-dirs (2026-07-15 → 2026-09-10), nested chat dirs including URL-derived slugs (`https-www-producthunt-com-products-tgdown`); corresponding threads exist in `state_5.sqlite` with these cwds.
- These hold **user-visible artifacts** (agent-written files), not session transcripts. They are user data — AgentTidy may *report* them (size, owning thread) but must treat them as **Blocked** for cleanup by default.
- Scan requirement: detection must cover `~/Documents/Codex/` in addition to `~/.codex/`, and correlate via `threads.cwd` to attribute space to sessions.
- Windows equivalent (unverified, assumed `%USERPROFILE%\Documents\Codex\`).

Related: `~/Documents/ChatGPT/` (153 MB in sample) is the analogous default workspace for the ChatGPT desktop app (contains full user projects with `.git`, `node_modules` — pure user data, Blocked). Not a v0.1 target provider, and v0.1 never scans it (detection only reads the three known providers' roots). If a future generic workspace-scan mode arrives, it will first need a "known non-target app" concept — the closed `ProviderId` set has no representation for it, and leaving the dir as an unknown structure would be exactly the misclassification to avoid. Deferred until then.

## Version discovery mechanism

- `codex --version` → `codex-cli 0.152.0`.
- `version.json`: `{"latest_version":"0.153.4","last_checked_at":…}` — self-update checker, not the installed version.
- Per session: `session_meta.payload.cli_version` (JSONL) and `threads.cli_version` (DB).

## Session identity & storage structure

- **Thread ID**: UUIDv7 (`019f…`). **Session (rollout) ID**: also UUIDv7; a rollout may have `parent_thread_id` ≠ its own `id` (subagent rollouts, `thread_source: "subagent"`, e.g. a `guardian` judge spawned from VS Code originator).
- **Rollout file**: `sessions/YYYY/MM/DD/rollout-<ISO-ts>-<uuid>.jsonl`, one per session, append-only. The **DB (`state_5.sqlite.threads`) is authoritative**: `threads.rollout_path` stores the absolute rollout path.
- **JSONL line types** (sample): `session_meta` (first line: `session_id`, `parent_thread_id`, `cwd`, `originator`, `cli_version`, `source`, `model_provider`), `turn_context` (per turn: `turn_id`, `cwd`, `workspace_roots`, `approval_policy`, `sandbox_policy`), `event_msg` (`task_started`, `token_count`, `task_complete`, `thread_settings_applied`, …), `response_item` (`message`, `reasoning`, …), `world_state`.
- `state_5.sqlite.threads` columns (key ones): `id`, `rollout_path`, `created_at`, `updated_at`, `source`, `model_provider`, `cwd`, `title`, `archived`, `archived_at`, `git_sha`, `git_branch`, `git_origin_url`, `cli_version`, `first_user_message`, `thread_source`, `originator`, `is_pinned`, `project_id` → `projects`.
- `thread_history_1.sqlite` = searchable projection: `thread_items(thread_id, turn_id, item_id, rollout_ordinal, item_json, …)`, `thread_turns(thread_id, turn_id, rollout_ordinal, status, duration_ms, …)`.
- `thread_spawn_edges(parent_thread_id, child_thread_id, status)` records parent/child relations.

## Archive / lifecycle semantics

- **Real archive exists**: `threads.archived` flag (5 of 96 in sample) + `archived_at` timestamp; archived rollouts are *moved* to `~/.codex/archived_sessions/` (flat layout, filename unchanged).
- `state_5.sqlite.rollout_migration_state` / `rollout_migration_skipped_rollouts` track a rollout→DB migration — schema versioning lives in `_sqlx_migrations` per DB.

## Project mapping

- Two levels: `threads.cwd` (absolute path) and optional `threads.project_id` → `projects` table (user-defined named projects, 6 in sample; `position`, metadata JSON).
- Rollout line-level: `session_meta.payload.cwd` and `turn_context.payload.workspace_roots[]`.
- No directory-slug filesystem mapping — mapping is DB-only. ⇒ Project views come from `state_5.sqlite`, not from the tree.

## Resource dependency graph

```
~/.codex
├── sessions/YYYY/MM/DD/rollout-*.jsonl      (per-session; path recorded in state_5.threads)
├── archived_sessions/rollout-*.jsonl       (same file, archived state)
├── state_5.sqlite
│   ├── threads (id → rollout_path, cwd, project_id, archived)
│   ├── projects
│   ├── thread_attachments (thread_id → attachments/<id>)
│   └── thread_spawn_edges (parent ↔ child)
├── thread_history_1.sqlite (projection of rollout content; thread_id keyed)
├── logs_2.sqlite (runtime logs; thread_id column)
├── session_index.jsonl (UI index: thread id + name)
└── attachments/<uuid>/                     (per-thread blobs)
```

Deleting a rollout file **without** DB consistency breaks: `threads.rollout_path` dangling, `thread_items` orphaned, `session_index` stale. ⇒ Cleanup must treat (rollout file + DB rows + projection rows + index lines) as one consistency set (red line #7/#8: no direct row deletion; executor protocol needed).

## Caches, logs, checkpoints

- `logs_2.sqlite` is the single largest safe-ish win (145 MB) but is append-only logging with `thread_id` references — row deletion is a DB mutation, so it must be its own reviewed cleanup unit, not free space.
- `cache/`, `models_cache.json`, `shell_snapshots/`, `.tmp/` — cache class.
- `thread-writer-locks/` — active writer locks; **never** clean while present.

## Databases & schema

- 5 SQLite DBs (`state_5`, `thread_history_1`, `logs_2`, `queue_1`, `memories_1`, `goals_1`), all WAL-mode with `-shm`/`-wal` sidecars, all with `_sqlx_migrations` (sqlx). Numbered suffixes (`state_5`) indicate **generation-bumped schemas** — old generations may coexist.
- Read access must be read-only (`mode=ro&immutable=1` URI or copy) and must tolerate WAL sidecars.

## Runtime write behavior

- WAL files actively written (`logs_2.sqlite-wal` mtime = now). Codex (CLI + VS Code extension + app server) writes concurrently.
- JSONL rollouts append with `thread-writer-locks` held during writes.

## Atomic cleanup units

- ✅ Candidate unit: archived thread = `archived_sessions/rollout-*.jsonl` + `state_5.threads` row + `thread_history_1` rows + `session_index` line — **but** DB row deletion violates red line #8 as "trash"; requires Codex-native archive/delete capability or a validated DB-transaction plan. v0.1 should stay read-only here.
- ✅ Safe pure-FS unit: nothing yet confirmed. `..codex-global-state.json.tmp-*` orphans are filesystem-only garbage (no DB refs) — small but trivially safe.
- ❌ Not units: any single SQLite file (contains cross-thread state), `cache/` only after write-behavior confirmation.

## Known risks

- **DB consistency**: naive file deletion dangles `threads.rollout_path`. Any cleanup plan must revalidate DB state.
- **Live WAL writers**: opening a DB read-write or deleting during writes corrupts state.
- **Orphaned tmp files** (`..codex-global-state.json.tmp-*`) show the atomic-write pattern fails sometimes — cleanup logic must not treat "tmp suffix" as unknown garbage blindly; verify no `.json` counterpart is newer.
- Subagent rollouts have `parent_thread_id`; deleting a parent must consider children (spawn edges).
- `~/Documents/Codex/` workspaces mix agent-generated files with user-moved/user-edited content — never clean wholesale; attribute-only (Blocked).

## Capability degradation rules

- Missing `state_5.sqlite` ⇒ no thread index ⇒ read-only listing from JSONL tree only (degraded, no archive info).
- `threads.cli_version` older than min-supported ⇒ mark Unknown → Blocked.
- Rollout file present but first line not `session_meta` ⇒ Unknown → Blocked.
- `archived=1` but file not in `archived_sessions/` (or vice versa) ⇒ inconsistent ⇒ Blocked until reconciled.

## Installation & data locations (Windows)

**Verified**: Windows sample collected 2026-09-20 (codex-cli 0.145.0; sample sessions written by 0.133.0 – 0.145.0 line).

Root: `%USERPROFILE%\.codex\` (e.g. `C:\Users\18712\.codex\`). Unlike Claude Code, Codex keeps **both** JSONL rollouts and several SQLite databases.

| Path | Role | Size (sample) | Cleanup class |
|---|---|---|---|
| `sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` | Session rollouts (append-only JSONL) | 208.64 MB / 19 files | Per-session |
| `archived_sessions/rollout-*.jsonl` | User-archived rollouts (flat) | — | Per-session (archived) |
| `state_5.sqlite` (+ `-wal`/`-shm`) | **Thread index**: `threads`, `projects`, `thread_attachments`, … | 0.84 MB | Index — never delete rows directly |
| `thread_history_1.sqlite` (+wal) | Projection of rollout content (`thread_items`, `thread_turns`) | 135.18 MB | Index |
| `logs_2.sqlite` (+wal) | Append-only runtime logs (`logs` table) | 45.24 MB | Log cache class |
| `queue_1.sqlite`, `memories_1.sqlite`, `goals_1.sqlite` | Queue / memory / goals features | ≤0.1 MB each | Feature state |
| `session_index.jsonl` | UI thread list: `{id, thread_name, updated_at}` | — | Derivable index |
| `attachments/<uuid>/` | Per-thread attachment blobs | 0.08 MB | Per-thread |
| `cache/`, `.tmp/`, `shell_snapshots/`, `thread-writer-locks/`, `ipc/` | Caches / locks | 26.95 MB | Cache class |
| `config.toml`, `auth.json`, `installation_id`, `version.json`, `hooks.json`, `rules/` | Config / credentials | — | Never clean |
| `..codex-global-state.json.tmp-*` (14 files) | **Orphaned temp write leftovers** in `%USERPROFILE%\.codex\` root | ~1 MB | Garbage (see risks) |
| `models_cache.json`, `cc-switch-model-catalog.json` | Model catalog caches | — | Cache |
| `archived_sessions/` sibling dirs (`agents/`, `automations/`, `browser/`, `computer-use/`, `generated_images/`, `goals_1.sqlite`…) | Feature data | — | Not yet classified |

### Default workspace directories (Windows)

Codex Desktop / Codex app creates **default per-day workspace dirs under `%USERPROFILE%\Documents\Codex\`** when the user starts a conversation without picking a folder:

```
%USERPROFILE%\Documents\Codex\YYYY-MM-DD\<chat-name>\        # e.g. 2026-09-10/new-chat/
├── work/                                       # agent working files
└── outputs/                                    # produced artifacts
```

- Verified in sample: 4 day-dirs (2026-08-03 → 2026-09-13), nested chat dirs including URL-derived slugs; corresponding threads exist in `state_5.sqlite` with these cwds.
- These hold **user-visible artifacts** (agent-written files), not session transcripts. They are user data — AgentTidy may *report* them (size, owning thread) but must treat them as **Blocked** for cleanup by default.
- Scan requirement: detection must cover `%USERPROFILE%\Documents\Codex\` in addition to `%USERPROFILE%\.codex\`, and correlate via `threads.cwd` to attribute space to sessions.

**Windows-specific notes**:
- Path separator: `\` (backslash) instead of `/` (forward slash) on macOS.
- Cross-drive cwds: possible (e.g. `D:\Projects\...`), but not observed in sample.
- Default workspace root: `%USERPROFILE%\Documents\Codex\` (confirmed).
- Case sensitivity: Windows is case-insensitive; compare paths case-insensitively.

**Scan locations for Codex on Windows** (deduplicated, case-insensitive):

| Location | Content | Role in app |
|---|---|---|
| `%USERPROFILE%\.codex\` | transcripts, DBs, caches (this doc) | State root — primary scan target |
| `%USERPROFILE%\Documents\Codex\` | default scratch workspaces + named workspaces | Workspace root — report-only (Blocked) |

## Open questions (Phase 0 continues)

- [x] Windows path (`%USERPROFILE%\.codex` verified 2026-09-20; also whether the default workspace is `%USERPROFILE%\Documents\Codex\` on Windows — verified).
- [x] Does Codex expose a native "delete thread" that also fixes DB? **Yes** — `codex delete <SESSION>` (verified in codex-cli 0.152.0, `codex delete --help`: "Permanently delete a saved session by id or session name"). This is the Application-layer's preferred path for `CleanupAction::ProviderOperation`; the §12.4 self-implementation (backup → BEGIN IMMEDIATE → DELETE → FK check) remains as a fallback when the binary is unavailable, fails, or behaves unexpectedly. Both paths are documented in [`../safety/provider-operation.md`](../safety/provider-operation.md).
- [ ] `logs_2.sqlite` rotation/retention policy (does Codex ever prune it?).
- [ ] Behavior of `attachments/` when its thread is archived (moved or left?).
- [ ] When are `%USERPROFILE%\Documents\Codex\YYYY-MM-DD\<chat>\` dirs created vs a user-picked cwd? (default-workspace trigger conditions)
