# Changelog

All notable changes to AgentTidy will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **The Windows desktop app runs and packages.** The app starts on
  Windows, shows the real scan state, and renders the overview, session
  list, review and diagnostics views against actual local data. Both
  Windows installers build: an NSIS setup executable and an MSI.
- **Windows support reaches the Mac feature level.** The five read-only
  providers are now discoverable on Windows where a real sample exists:
  Antigravity joins Claude Code (CLI/Desktop), Codex and WorkBuddy. The
  Antigravity state root, index database and its three verified trees are
  structurally identical to the macOS sample, and the adapter's projection
  query runs unchanged against the Windows database.
- **Windows native cleanup acceptance.** A strict Recycle Bin round-trip
  test proves that a file, a nested directory and paths containing spaces,
  CJK characters and quotes survive a real trash move and a manual restore,
  with the file identity and the bytes unchanged. The Windows Recycle Bin
  keeps the original path inside its `$I` metadata file rather than the
  file name, and the test reads it from there.
- **A controlled Phase 7 cleanup acceptance on real files.** A synthetic
  provider installation — real directory layout, real database, real files —
  is driven through the whole loop: discovery, planning, revalidation,
  execution into the real OS trash, an idempotent re-run and a manual
  restore with a byte-for-byte check. It only ever touches its own
  uniquely named sample.
- A Windows scenario page recording this machine's own results, kept
  deliberately separate from the macOS one: a macOS result is never
  reported as a Windows result.

### Fixed

- **Windows CI is green again.** Four platform-specific defects failed the
  Windows job: a `mut` binding that only unix needed (breaking clippy),
  a temporary database directory that could not be deleted while the
  connection still held a file lock, a provider contract test that
  deleted its fixture before closing the handle, and a frontend test that
  needed a directory symlink — Windows now uses a junction, which needs
  no elevation.
- **Allocated size is reported again, on every platform.** The usage
  accumulator started from "unknown", so the first measurement collapsed
  it back to "unknown" and the allocated-size figure was never reported for
  any scope. A fully measured scope now reports its allocated total, and a
  single unmeasurable file still makes the total unknown.
- **Read-only SQLite picks the right access tier.** Opening a WAL database
  whose shared-memory file is missing used to succeed lazily — the failure
  only appeared on the first real statement, so the private-copy fallback
  never ran and providers silently skipped their inference. The open is
  now verified before it is handed out, and a file that is not a database
  is rejected instead of being read as an empty one.
- **Exclusive ownership is provable on Windows.** Hard-link count was
  unreadable outside unix, so no Windows session could ever be proven
  exclusively owned and no cleanup unit could be built. Windows now reads
  the link count natively.
- **Windows workspace URIs decode correctly.** A workspace URI with a
  drive letter (`file:///c%3A/...`) now resolves to a usable Windows path,
  and an empty session title degrades to "no title" instead of rendering
  blank.

### Added

- macOS read-only Antigravity and Pi adapters, grounded in local metadata samples:
  discovery, session/project listing, file usage and diagnostics through the shared
  API, CLI and desktop. Antigravity multi-root associations remain visible without
  arbitrary project attribution. Archive, cleanup and recovery remain unsupported.
- Sanitized provider fixtures, WAL/schema/link boundary regressions and CLI coverage;
  a shared desktop provider-label catalog prevents new sources inheriting another label.

- **Phase 7 first cleanup slice — Claude Code transcript `FileSet` beta.**
  `build_cleanup_units` / `validate_cleanup_unit` now emit and validate
  atomic trash units (transcript + same-named side directory) for
  recognized, exclusively-owned sessions, gated by the new
  `docs/safety/session-transcript-fileset.md`; unrecognized/mismatched
  transcripts never form units; other resource types have their own
  Phase 7 gates. Supporting pieces: multi-path
  `trash::move_paths_to_trash` with honest §12.3 partial-failure
  aggregation; the §12.1 cross-process operation guard
  (`$HOME/.agenttidy/operations.lock`, OS advisory lock with crash
  release); `validate_cleanup_unit` now takes the installation so
  §16.1 containment (canonicalized data-root boundaries, symlink-escape
  rejection, verified `projects/<slug>/` layout) is actually enforceable;
  `CleanupUnit`/`CleanupItem` carry a typed `CleanupLocator` (the full
  `FileSet` path set travels to the executor and audit log) plus
  `estimated_reclaimable_bytes` (the GUI byte totals now use the unit
  estimate instead of the primary member's fingerprint size).
- Typed §11.1/§11.2 lifecycle rules in `cleanup_policy_evaluate`:
  Active/Unknown sessions are Blocked, `Archived + inactive > 30d +
  exclusive + recoverable` is the single LowRisk path (recommended), and
  Inactive / recently-archived units stay ReviewRequired — units without
  snapshot linkage, non-exclusive ownership or dependencies are Blocked
  (§7.3 / §9.5).
- §6.3 Review & Tidy completeness + selection-rule hardening (GUI):
  every plan row now shows its operation mode (Move to Trash / Provider
  operation), recoverability (system-trash recoverable vs not) and the
  contained member paths of a `file-set` unit; blocked rows are
  non-selectable at the state layer (not just the checkbox), stale
  (changed) rows drop out of the selection at revalidation as the copy
  promises, and the view is reset after execution so the next run
  requires a fresh preview. Plan expiry (5-min TTL) now gates both CTAs
  with a "plan expired — rescan" notice, checked live at click time.
- **Phase 7 second cleanup slice — Codex archived-session
  `ProviderOperation` candidate preview (execution disabled)** (gated by the new
  `docs/safety/provider-operation.md`): `AgentProviderAdapter` gains
  `execute_provider_operation` (default `Failed("not supported")`) and
  the executor dispatches `CleanupAction::ProviderOperation` items to
  it. Codex emits one `DbRow`-locator unit per archived, exclusively-
  owned session; candidates remain Blocked pending verified native deletion
  and complete recovery. Native deletion and automatic DB replacement are
  disabled, never branded "Move to Trash" (§12.4 / red line #8).
  `inspect()` surfaces `state_5` `user_version`, schema SQL
  and journal mode for revalidation.
- **Phase 7 third cleanup slice — WorkBuddy session `FileSet` beta**
  (gated by the new `docs/safety/workbuddy-session-fileset.md`; the
  `sessions` table contract is frozen as v1 in
  `fixtures/workbuddy/db/sessions-schema.json`): scan now derives
  session lifecycle from the database — `deleted_at IS NOT NULL` ⇒
  `Archived`, recognized terminal statuses ⇒ `Inactive`, anything
  else (including a missing/drifted/unreadable DB) stays `Unknown`
  fail-closed. Cleanup units are file-only `Trash` actions for
  Inactive/Archived sessions (transcript + side dir + `.meta.json` +
  `.file-rollback.ndjson`; no DB rows are mutated) with a
  plan-build cross-check that the `sessions` row still maps to a
  cleanable lifecycle. `automations.cwds` is read (contract v1) to
  expose `automation_cwd_refs` on the workspace resource, and the
  shared §16.1 path-safety block lives in
  `infrastructure::paths::validate_transcript_fileset_paths`.

- Local Mac scenario coverage: session search / Agent, lifecycle and age
  filters / size and activity sorting / 50-row pagination; frontend regression
  tests now run in CI. Cleanup requests serialize, ignore cancelled responses,
  expire confirmation live, and require a fresh preview after any execution
  attempt. Provider operations have action-appropriate confirmation copy.

### Changed

- New graphite/silver/emerald desktop identity with a sculptural archive mark,
  exported from committed transparent artwork to ICNS, ICO, desktop PNGs and
  favicon. The workspace header and startup screen share the same icon.
- Brand startup page follows real scan state, allows background browsing and
  exposes retry/Diagnostics after failure. It exits automatically when ready,
  respects reduced motion and can be reopened from the workspace icon.

- Mac desktop overview now leads with identified storage, agent distribution,
  diagnostics and direct links to the largest sessions. Session details expose
  linked resources and backend cleanup conditions without loading message bodies.
- Cleanup review prioritizes session/project identity and lifecycle explanations;
  paths, plan IDs and technical reasons remain available on demand. Current-attempt
  receipts show only submitted items and explain manual Trash recovery, original
  locations and partial failures. Identity labels survive the post-cleanup scan.
- Read-only and cleanup request hooks are separate modules. Cleanup preview is
  generated on the first Review visit instead of adding a third scan at startup;
  navigation does not cancel an execution or discard session filters.

- Codex cleanup stays Blocked until complete database/transcript/index/
  attachment recovery is verified. The native mutation entry point is disabled.
  Read-only CLI admission checks run in preview and Diagnostics, scoped to
  the detected `CODEX_HOME`; a supported CLI alone cannot enable cleanup.

### Fixed

- Codex thread timestamps are converted from verified Unix seconds to domain
  milliseconds, fixing 1970 dates, ordering and age filters. Native CLI probes
  have a three-second deadline and kill the child on cancellation so desktop
  launchers cannot hold the first scan indefinitely.
- Overview diagnostics merge inspection and scan codes without double counting;
  inspected warnings are no longer hidden whenever a snapshot is present.

- Shared-database selections are rejected in full before any mutation instead
  of invalidating later rows after deleting the first session. Canonical aliases
  and hard links cannot bypass the restriction; use one DB session per fresh
  preview until a coordinated provider batch contract is validated.
- Missing, failing and unsupported Codex CLIs are reported before confirmation;
  recovery and CLI blockers have explicit English/Chinese preview explanations.

- macOS Trash uses NSFileManager instead of Finder AppleScript. Strict local
  file/directory move-and-restore acceptance covers CJK, spaces and quotes.
  Trash moves no longer claim estimated bytes as physical disk reclamation.
- WorkBuddy's macOS Electron executable is recognized from its bundle path,
  preventing a running WorkBuddy 5.8.3 from being treated as stopped.
- Cleanup uses an OS-held persistent-file lock with crash release, not age-based
  takeover. Audit sinks open before mutation, sync intent/result events and
  preserve new events after truncated crash lines. Unsafe preview units become
  Blocked rows without hiding healthy units; blocker reasons appear first.
- Codex native deletion remains disabled: the isolated protocol experiment did
  not establish successful deletion or complete recovery on the local Mac.
  Neither native execution nor database replacement is enabled by a CLI probe.

- Cleanup execution rediscovers installations across independent IPC calls;
  Codex database fingerprints now use DB metadata throughout. Execution
  holds the operation guard before fresh per-item scans, re-applies policy
  and provider validation, compares schema/journal and complete file-set
  metadata evidence, and binds full item payloads/expiry to confirmation.
  Selected stale/blocked items are audited without mutation.
- SQLite backups run `VACUUM INTO` on the read-only source, include live WAL
  frames in a standalone snapshot and refuse to overwrite recovery points.
  The torn raw-copy fallback has been removed. Headless Trash regressions
  no longer assume that a failed move removed the original files.

- Windows locked-file classification (§16.2 / §12.3): a member held
  open by another process is now recognized from the platform error
  code (`ERROR_SHARING_VIOLATION` / `ERROR_LOCK_VIOLATION`, raw and
  HRESULT forms) as `TrashOutcome::Locked` — a locked-only failure
  with nothing moved reports `skipped` with a close-the-holder-and-retry
  hint instead of a bare `failed`, while a lock arriving after other
  members already moved keeps the honest `failed` (a partially-moved
  file set is never a clean skip).

- The cleanup executor now actually enforces the plan TTL it documents:
  `cleanup_revalidate` marks every item of an expired plan `skipped` and
  `cleanup_execute` refuses to run it ("plan expired — rescan"), closing
  the gap where a stale plan could still execute via raw IPC despite
  `Start.md` §12.2.

- Desktop application icon: a reproducible 1024×1024 source icon
  (`apps/desktop/assets/app-icon.png`, drawn by
  `apps/desktop/assets/generate_app_icon.py`) — rounded teal→emerald plate with
  a white "tidy rows + sparkle" glyph matching the GUI's emerald accent — with
  the full platform icon set (`icon.icns` / `icon.ico` / all PNG sizes,
  `apps/desktop/src-tauri/icons/`) generated via `tauri icon`; the web shell
  serves a matching favicon.

- Adaptive, terminal-friendly Unicode tables for the human-facing CLI output,
  with readable byte units and status colours; `--json` remains unchanged for
  automation.
- Versioned JSON envelopes (`agenttidy.cli.v1`) for every CLI machine-output
  command, including its command name and explicit `read-only` mode.
- Pure application-level workspace preview admission checks with regression
  coverage for the exact-session and WorkBuddy automation-reference gates;
  this is reserved for the future GUI and deliberately not exposed by the CLI.
- Initial Codex read-only provider adapter: detects the shared Codex state
  root, reports inspection facts and scans rollout JSONL `session_meta`
  records without reading transcript bodies. It reads the documented
  `state_5.sqlite.threads` index read-only to enrich session metadata; an
  unavailable or inconsistent index degrades diagnostics, while malformed
  rollouts remain blocked by default.
- Initial Claude Code CLI read-only provider adapter: discovers the verified
  `~/.claude` root and scans only top-level project transcripts. A transcript
  and its same-named side directory form one measured Session resource;
  filename/session-id mismatches remain blocked by default.
- Initial WorkBuddy read-only provider adapter: scans only the verified
  transcript file set and reports database journal facts without querying
  session lifecycle. Claude Desktop is discovered as a second Claude Code
  installation and scans only embedded transcript mirrors, excluding VM,
  credential, audit and user-output paths.
- Phase 4 read-only application/CLI path: static registration of the three
  providers and functional `doctor`, `scan`, and `sessions` commands. Cleanup
  is intentionally a future GUI-only workflow, not a CLI subcommand.
- End-to-end CLI golden baselines: 12 committed snapshots under
  `apps/cli/tests/golden/` (3 providers × 3 commands + 3 empty-home
  regressions) lock the `agenttidy.cli.v1` envelope against the existing
  sanitized fixtures. Absolute paths, completion timestamps, the
  agent-running flag, session lifecycle state and the `platform` label are
  normalised to placeholders so a single golden set covers macOS and
  Windows CI runners; the locked contract includes `thread-index-unavailable`
  on Codex scans, `workbuddy-db-unavailable` on WorkBuddy inspections and
  the four workspace-safety booleans on default-workspace resources.
- Phase 5 GUI read-only experience: Tauri 2 IPC bridge
  (`apps/desktop/src-tauri/src/commands.rs`) forwarding `doctor` /
  `scan` / `detect` to `agenttidy-application`, plus a three-view React
  shell (Overview / Sessions / Diagnostics) per Start.md §6.1/§6.2/§6.4
  with hand-written TS types in `apps/desktop/src/types.ts` matching the
  Rust serde shapes. Auto-scan on mount plus a manual Refresh button;
  empty installs route to a dedicated `Empty` panel. Cleanup actions
  (§6.3 Review & Tidy) remain a Phase 6 deliverable.
- Phase 6 cleanup model (framework-only): typed domain vocabulary in
  `crates/core/src/cleanup.rs` (three-level RiskLevel, CleanupUnit /
  CleanupPlan / CleanupItem, §12.2 revalidation precondition kinds,
  tagged CleanupLocator + §16.3 CleanupEvent); `AgentProviderAdapter`
  gained `build_cleanup_units` / `validate_cleanup_unit` (default: empty
  — no provider is cleanup-enabled until Phase 7); Application API
  gained the pure `cleanup_policy_evaluate`, `cleanup_plan_from_snapshots`,
  `cleanup_revalidate`, and fingerprint-guarded `cleanup_execute`; an
  append-only fsynced audit log at `$HOME/.agenttidy/operations.jsonl`;
  `agenttidy.gui.v1` IPC envelopes over three Tauri commands
  (`cleanup_preview` / `cleanup_revalidate` / `cleanup_execute`); and the
  GUI's Tidy tab implementing the two-confirmation flow with the Mac
  cleaner review layout (Recommended / Caution / Off-limits badges,
  per-provider groups, typed CONFIRM modal). `docs/safety/workspace-cleanup.md`
  now enumerates the §12.2 revalidation triggers, the audit-log schema,
  and the Trash vs ProviderOperation boundary. The CLI stays read-only —
  no cleanup subcommand.
- GUI bilingual (zh/en) internationalization: a hand-rolled i18n layer
  (`apps/desktop/src/i18n/`) with `en.ts` as source of truth and `zh.ts`
  typed as `Record<TranslationKey, string>` — missing/extra keys fail
  `tsc -b`, so `pnpm build:desktop` is the catalog-completeness gate.
  First launch detects `navigator.language`, the header carries an
  EN / 中文 toggle persisted in `localStorage`, and `<html lang>` follows
  the active locale. Static UI, frontend-mapped enum labels (status /
  capability / lifecycle / risk / confidence / severity / outcome) and
  the Phase 7 banner are fully localized; backend `ScanProblem` messages
  map to Chinese by stable code/path-suffix with English fallback, while
  cleanup reasons and anyhow errors stay English until Phase 7. Shared
  `format.ts` consolidates the previously copy-pasted `humanBytes` and
  locale-formats the three timestamp columns.
- Documented the future default-workspace cleanup gate: exact exclusive
  session ownership, no Git repository or shared references, OS Trash only,
  revalidation, risk disclosure, and two independent user confirmations.
- Added read-only accounting for the verified Codex and WorkBuddy default
  workspace roots; CLI scan reports their footprint separately from sessions.
- Development progress changelog at `docs/CHANGELOG.md`: a Phase 0–8
  milestone view with current work, verification blockers and maintenance
  conventions; README now links to it.
- Initial project skeleton: Rust workspace (core, application, infrastructure,
  provider-api, test-support crates; workbuddy / claude-code / codex provider
  crates; `agenttidy` CLI with placeholder subcommands), Tauri 2 + React
  desktop app shell, pnpm workspace, CI (macOS / Windows), docs and provider
  investigation placeholders.
- Provider investigation (Phase 0): filled `docs/providers/{claude-code,codex,workbuddy}.md`
  from real macOS samples — session storage structures, SQLite schemas,
  resource dependency graphs, archive semantics, atomic cleanup unit candidates
  and known risks. AGENTS.md + CLAUDE.md instruction files.
- Provider investigation update: default agent workspace directories outside
  state roots (`~/Documents/Codex/`, `~/WorkBuddy/` incl. nested `.workbuddy/`
  agent state) — detection must scan multiple common locations (Documents,
  home, app install), workspaces classify as Blocked/report-only; WorkBuddy
  user-chosen cwds with spaces/CJK verified; per-session sidecar files
  (`<uuid>.meta.json`, `<uuid>.file-rollback.ndjson`) documented.
- Provider investigation update (Windows 10, 2026-09-20): verified state
  roots for all three in-scope providers. New
  `docs/providers/claude-desktop.md` documents Claude Desktop
  (`%LOCALAPPDATA%\Claude-3p\`, ~10 GB dominated by a Linux VM rootfs) as
  the **second `claude-code` installation** (not a separate `ProviderId`).
  `docs/providers/windows-path-matrix.md` records the per-installation
  path matrix. Codex CLI + Codex Desktop share one state root and are
  modelled as a single `codex` installation. `docs/providers/phase0-windows-{summary,complete,final}.md`
  archive the full investigation.
- Sanitized Phase 0 fixtures (`fixtures/<provider>/config/directory-tree.json`
  + `fixtures/<provider>/sessions/sample-session.jsonl` for each of
  `workbuddy`, `claude-code`, `codex`). Directory skeletons are depth-1
  with zero paths/PII; session JSONLs preserve schema fields and line
  types but drop all message content, paths, identity files, trace IDs
  and usage payloads. UUIDs are kept because they are schema-relevant
  for session-link tests.

### Fixed (Phase 1 review, installation-model alignment)

- Native-trash test now skips its environment-dependent assertion when a
  headless macOS session has no Finder service; production code still returns
  the platform error and remains fail-closed.
- `SessionId` doc corrected: uniqueness is per *installation*, not per
  provider — the same id may legitimately appear in both `claude-code`
  installations (Desktop mirrors CLI transcripts); dedup by
  `(provider, session id)` is the application layer's job.
- `AgentInstallation.id` now documents the `<provider>:<role>` naming
  convention; test fixtures renamed `claude-code:default` →
  `claude-code:cli` to match it (5 occurrences).
- Removed unused `Session::CAPABILITY_TOPIC` const and the
  keep-import-alive `unused_topic_import_is_for_docs` test.

### Fixed (Windows test-environment isolation)

- CLI golden baselines now redirect `LOCALAPPDATA` at the temp home: the
  Windows Claude Desktop installation is discovered via
  `LOCALAPPDATA/Claude-3p`, so a real machine install previously leaked
  into every golden comparison and broke the empty-home regressions
  outside CI.
- Golden comparison normalises measured `allocated_bytes` to a
  placeholder (`null` "not measured" stays verbatim): on-disk allocation
  depends on host cluster size / NTFS compression and cannot be stable
  across dev machines and CI runners.
- Audit-log tests now switch the env var `paths::home_dir` actually
  reads (`USERPROFILE` on Windows, `HOME` elsewhere) instead of `HOME`
  alone, so they no longer write against the real user profile on
  Windows.

### Added (Phase 1: read-only core)

- `agenttidy-core` domain model: `ProviderId` (closed set, fail-closed parse),
  `AgentInstallation`/`Platform`/`InstallationStatus`, `CapabilityStatus`/
  `CapabilityTopic`/`AgentCapabilities` (missing topic ⇒ Unsupported),
  `Session`/`SessionId`/`ProjectRef`/`SessionLifecycle` (archived-as-signal),
  `Resource`/`ResourceKind`/`Ownership`/`ResourceLocator`/`ManagedBy`,
  `SizeInfo`/`SizeConfidence` (conservative merge, no fake precision),
  `AgentSnapshot`/`ScanOptions`/`ScanProblem`, static `ProviderRegistry`
  (duplicate registration is a hard error).
- `agenttidy-provider-api`: `AgentProviderAdapter` trait
  (detect/inspect/capabilities/scan, read-only) and `ProviderInspection`
  (§10.1 pre-scan facts: schema versions, journal modes, agent-running flag,
  unknown structures). Cleanup-unit methods intentionally deferred to Phase 6.

### Fixed (Phase 1 code review)

- `ProviderId` now validates on `Deserialize` as well as on `parse` — the
  closed-set invariant holds on the wire/IPC path, not just in-process
  construction (previously any string round-tripped through serde).
- `AgentProviderAdapter` is now dyn-compatible (via `async-trait`: boxed
  `Send` futures, `Send + Sync` supertraits) — the documented
  `ProviderRegistry<Box<dyn AgentProviderAdapter>>` wiring now compiles.
- `SizeInfo::merge` confidence now follows the weakest contributor: any
  unmeasured (`Unknown`) side makes the aggregate `Unknown` instead of
  `Estimated`, matching `opt_add`'s no-partial-sums stance.
- `AgentSnapshot::total_logical_bytes` replaced by `total_size()` returning
  `SizeInfo` — totals carry confidence and no longer silently count
  unmeasured resources as trustworthy zeros.
- `SessionLifecycle::Archived.archived_at` is `Option<u64>` (epoch ms)
  instead of `Option<serde_json::Value>`, matching `created_at`/`updated_at`
  and enabling the §11.1 "archived + inactive > 30d" rule.
- Removed `Session::supports_archive` (core-side hardcoded provider list) —
  archive support is a provider-reported fact via `CapabilityTopic::Archive`.
- `CapabilityStatus` and `InstallationStatus` now derive `Ord` with
  worst-to-best declaration order, making their documented ordering claims
  real (`max()` picks the strongest).
- Registry doc corrected: duplicate registration returns
  `Err(DuplicateProvider)`, it does not panic.
- `docs/providers/codex.md`: ChatGPT-dir recognition deferred — v0.1 only
  scans known provider roots, and the closed `ProviderId` set has no
  "known non-target app" representation yet.

### Added (Phase 2: infrastructure)

`crates/infrastructure` is now live (was a 7-line placeholder). The
crate exposes provider-neutral platform services with platform code
confined to `#[cfg]` branches inside its modules (red line #11).
Everything in Start.md §13.1 is implemented:

- **Path safety** (`paths`): `safe_canonicalize` (verbatim-stripped
  Windows canonical form), `strip_verbatim` (`\\?\C:\x` ↔ `C:\x`,
  `\\?\UNC\…` ↔ `\\…`), `normalize_drive_letter` (`c:` → `C:`),
  `is_within` (component-based containment, strict-descendant,
  Windows case-insensitive / macOS case-sensitive — the asymmetry is
  deliberately the conservative direction per §16.1), and
  `has_reserved_component` (`CON`/`PRN`/… — Windows-only).
- **Read-only filesystem probing** (`fs_probe`): `walk_tree` over
  `walkdir` with `follow_links(false)` — symlinks/junctions are
  returned as `EntryKind::Link` (never followed), Windows cloud
  reparse-point dirs and Unix special files are `EntryKind::Unknown`
  (caller fails closed). Each file carries `FileIdentity` (Windows
  `BY_HANDLE_FILE_INFORMATION` via `CreateFileW` with `FILE_FLAG_OPEN_REPARSE_POINT`;
  Unix `dev`/`ino`) and `allocated_len` (Windows `GetCompressedFileSizeW`,
  Unix `st_blocks * 512`).
- **Size aggregation** (`disk_usage`): `UsageAccumulator` dedupes
  hard links by identity (§8), reports
  `hardlink_duplicates_skipped` and `unidentifiable_files`, and drops
  `allocated_bytes` to `None` on any unmeasured contribution (no
  fake precision).
- **Streaming JSONL** (`jsonl`): `open_jsonl`/`JsonlReader` — per-line
  events (`Item`/`Malformed`/`Fatal`); empty lines and parse errors
  are surfaced, never abort the stream.
- **Read-only SQLite** (`sqlite`): `ReadOnlyDb::open` does `mode=ro`
  URI open and falls back to a **private temp copy** of `db` + `-wal`
  + `-shm` opened rw on *our* copy when WAL recovery or a busy
  lock blocks the direct read. `journal_mode` and `quick_check_ok`
  surface §10.1 inspection facts. URI encoder percent-encodes
  provider-realistic paths (spaces, CJK).
- **Process signals** (`processes`): `ProcessSignature`
  (`exe_names` + `cmdline_substrings`) and `any_process_running`,
  driven by `sysinfo` 0.30 — deliberately narrow, never a decision
  (§16.2: one signal among several).
- **Trash** (`trash`): `move_to_trash` wrapping `trash` crate (Windows
  Recycle Bin / macOS NSWorkspace). Missing path = `TrashError::NotFound`
  (fail closed); locked files surface as `TrashError::Platform` for the
  Phase 6 executor to mark `skipped`.

Tests: 31 in infrastructure (31 pass on Windows). Workspace total:
55 tests pass. `cargo fmt` and `cargo clippy --workspace --all-targets
-- -D warnings` clean.

New workspace deps: `walkdir`, `rusqlite` (bundled), `trash`, `sysinfo`,
`windows-sys` (Win32_Storage_FileSystem, Win32_Foundation, Win32_Security —
last one gates `CreateFileW`'s `SECURITY_ATTRIBUTES` signature).
