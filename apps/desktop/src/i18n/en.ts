// English catalog — the source of truth for every user-facing string in
// the desktop GUI. `TranslationKey` (derived from this object) is the
// single key union: `zh.ts` is typed as `Record<TranslationKey, string>`,
// so a missing or extra key fails `tsc -b` — `pnpm build:desktop` is the
// catalog-completeness gate (i18n plan, Phase 5/6 GUI).
//
// Conventions:
// - `{name}` placeholders are substituted by `t(key, params)`.
// - English plurals use `.one` / `.other` key pairs; Chinese supplies the
//   same string for both (no plural rules).
// - Strings intentionally NOT translated live outside this catalog:
//   brand name (AgentTidy), the functional `CONFIRM` gate token, IEC
//   byte units, provider/platform brand ids, paths, ids, and backend
//   free-form text (cleanup reasons, anyhow errors) — those stay English
//   at the call site.
// - `problem.*` keys are zh-facing mappings for stable ScanProblem codes;
//   the English UI renders the backend message verbatim via fallback.

export const en = {
  // --- App chrome ---
  "tab.overview": "Overview",
  "tab.sessions": "Sessions",
  "tab.tidy": "Tidy",
  "tab.diagnostics": "Diagnostics",
  "header.tagline": "Local-first agent storage inspector · trash-first · fail-closed",
  "common.refresh": "Refresh",
  "common.scanning": "Scanning…",
  "common.failure": "Failed:",

  // --- Empty state ---
  "empty.title": "No supported agent installations detected",
  "empty.body":
    "AgentTidy looks for Codex, Claude Code, and WorkBuddy on this machine. Install one of them, or run with elevated permissions to see detected installations here.",

  // --- Overview ---
  "overview.noInstallations": "No installations detected yet. Try Refresh.",
  "overview.sessions": "Sessions",
  "overview.resources": "Resources",
  "overview.totalSize": "Total size",
  "overview.workspaceFootprint": "Workspace footprint",
  "overview.workspaceValue.one": "{count} resource · {bytes}",
  "overview.workspaceValue.other": "{count} resources · {bytes}",
  "overview.diagnostics.one": "{count} diagnostic →",
  "overview.diagnostics.other": "{count} diagnostics →",

  // --- Installation status badges (statusBadge) ---
  "status.available": "Available",
  "status.permissionRequired": "Permission required",
  "status.unsupported": "Unsupported",

  // --- Capability status badges (capabilityBadge) ---
  "capability.unsupported": "Unsupported",
  "capability.permissionRequired": "Permission required",
  "capability.degraded": "Degraded",
  "capability.readOnly": "Read-only",
  "capability.supported": "Supported",

  // --- Capability topic labels (rendered raw today) ---
  "topic.sessions": "sessions",
  "topic.projects": "projects",
  "topic.archive": "archive",
  "topic.cache": "cache",
  "topic.logs": "logs",

  // --- Size confidence (wire enum → label) ---
  "confidence.exact": "exact",
  "confidence.estimated": "estimated",
  "confidence.unknown": "unknown",

  // --- Session lifecycle (lifecycleLabel) ---
  "lifecycle.active": "Active",
  "lifecycle.inactive": "Inactive",
  "lifecycle.unknown": "Unknown",
  "lifecycle.archived": "Archived",

  // --- Provider / platform display names (brand ids, same in both langs) ---
  "provider.claudeCode": "Claude Code",
  "provider.codex": "Codex",
  "provider.workbuddy": "WorkBuddy",
  "platform.macos": "macOS",
  "platform.windows": "Windows",

  // --- Sessions view ---
  "sessions.empty.title": "No recognizable sessions",
  "sessions.empty.body":
    "The currently detected installations did not yield any sessions. This usually means the state directory is empty or no supported agent has been used on this machine yet.",
  "sessions.col.installation": "Installation",
  "sessions.col.session": "Session",
  "sessions.col.project": "Project cwd",
  "sessions.col.lifecycle": "Lifecycle",
  "sessions.col.size": "Size",
  "sessions.col.updated": "Last updated",

  // --- Diagnostics view ---
  "diag.empty": "Diagnostics appear once an installation has been detected.",
  "diag.clean":
    "All detected installations reported zero inspection warnings. Detection and inspection completed cleanly.",
  "diag.readableRoots": "Readable roots",
  "diag.readable": "readable",
  "diag.notReadable": "not readable",
  "diag.schemaVersions": "Schema versions",
  "diag.journalModes": "Journal modes",
  "diag.dataRoots": "Declared data roots",
  "diag.problems.one": "{count} problem",
  "diag.problems.other": "{count} problems",
  "severity.warning": "warning",
  "severity.error": "error",

  // Stable ScanProblem codes → canonical English (zh maps to Chinese; the
  // English UI keeps the backend message verbatim via fallback, so these
  // values are documentation of intent rather than rendered text).
  "problem.thread-index-missing": "state_5.sqlite missing; JSONL-only scan will be degraded",
  "problem.thread-index-unavailable": "thread index unavailable; JSONL-only scan",
  "problem.projects-missing": "projects directory missing",
  "problem.projects-unreadable": "projects directory is not readable",
  "problem.desktop-sessions-missing": "Claude Desktop has no local-agent-mode sessions",
  "problem.workbuddy-db-journal": "could not read workbuddy.db journal mode",
  "problem.workbuddy-db-unavailable": "workbuddy.db is unavailable",
  // Dynamic codes carry a path/id suffix appended verbatim after translation.
  "problem.unreadable": "unreadable path:",
  "problem.unrecognized-transcript": "unrecognized transcript (skipped):",
  "problem.unrecognized-rollout": "unrecognized rollout (skipped):",
  "problem.duplicate-session": "duplicate session id (left unmerged):",
  "problem.thread-index-missing-session": "rollout missing from the thread index:",
  "problem.archive-state-mismatch": "archive state disagrees with the thread index:",
  "problem.project-unreadable": "cannot stat project directory:",
  "problem.workspace-unreadable": "workspace path unreadable:",

  // --- Review (Tidy) view ---
  "risk.lowRisk": "Recommended",
  "risk.reviewRequired": "Caution",
  "risk.blocked": "Off-limits",
  "review.loading": "Loading cleanup plan…",
  "review.found.one": "Found {bytes} across 1 cleanup unit",
  "review.found.other": "Found {bytes} across {count} cleanup units",
  "review.selected": "Selected",
  "review.planId": "plan {id}",
  "review.expires": "expires {when}",
  "review.back": "Back",
  "review.cleanNow": "Clean Now",
  "review.reviewSelected": "Review Selected",
  "review.phase7.before":
    "No cleanup units in the latest scan. Provider cleanup rolls out per resource type (Start.md §19) — the Claude Code transcript cleanup beta is live; other resource types land as each passes its safety gate. Every resource type gets its own ",
  "review.phase7.after":
    " document before its cleanup is enabled.",
  "review.bucket.recommended": "{count} recommended",
  "review.bucket.caution": "{count} caution",
  "review.bucket.offLimits": "{count} off-limits",
  "review.executionResults": "Execution results",
  "outcome.pending": "pending",
  "outcome.removed": "removed",
  "outcome.skipped": "skipped",
  "outcome.failed": "failed",
  "review.stale": "changed — rescan",
  "review.revalidatePassed":
    "Revalidation passed — every selected unit is unchanged since the plan was built.",
  "review.revalidateChanged.one":
    "1 unit changed since the plan was built and was removed from selection (§12.2). Re-run Back → Refresh to rebuild.",
  "review.revalidateChanged.other":
    "{count} units changed since the plan was built and were removed from selection (§12.2). Re-run Back → Refresh to rebuild.",
  "review.ariaSelect": "select {id}",
  "review.expired":
    "Plan expired — please rescan (Back → Refresh) before reviewing or cleaning (§12.2).",
  "review.item.action.trash": "Move to Trash",
  "review.item.action.providerOperation": "Provider operation",
  "review.item.recoverable.trash": "Recoverable from the system Trash",
  "review.item.recoverable.providerOperation": "Not trash-recoverable",
  "review.item.members.one": "1 path in this unit",
  "review.item.members.other": "{count} paths in this unit",

  // --- ReviewConfirm modal ---
  "confirm.title": "Confirm cleanup",
  "confirm.body.one":
    "You are about to move 1 unit ({bytes}) to the system Trash. Recovery is handled by macOS / Windows — AgentTidy performs no permanent deletion.",
  "confirm.body.other":
    "You are about to move {count} units ({bytes}) to the system Trash. Recovery is handled by macOS / Windows — AgentTidy performs no permanent deletion.",
  "confirm.plan": "Plan",
  "confirm.fingerprint": "Fingerprint",
  "confirm.reclaimable": "Reclaimable",
  "confirm.typeBefore": "Type ",
  "confirm.typeAfter": " to enable the execute button.",
  "confirm.cancel": "Cancel",
  "confirm.moveToTrash": "Move to Trash",
} as const;

export type TranslationKey = keyof typeof en;