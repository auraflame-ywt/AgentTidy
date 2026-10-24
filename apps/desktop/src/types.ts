// TypeScript shapes for the AgentTidy desktop IPC contract.
//
// These mirror the Rust serde-derived shapes in:
//   - crates/core/src/{installation,session,resource,size,snapshot,capability,provider}.rs
//   - crates/application/src/lib.rs           (DoctorReport)
//   - crates/provider-api/src/lib.rs          (ProviderInspection)
//
// serde_json serializes object keys alphabetically (no `preserve_order`),
// so the field order on the wire is *not* the declaration order below;
// consumers should rely only on field names, never position. Tagged enums
// (`SessionLifecycle`, `ResourceLocator`, `ScanProblem`) serialize with the
// tag inlined — see the inline `// tagged:` comments.

// --- Provider & installation ---

/** `crates/core/src/provider.rs`: closed set, serde as bare string. */
export type ProviderId = "claude-code" | "codex" | "workbuddy" | "antigravity" | "pi";

/** `crates/core/src/installation.rs`: `#[serde(rename_all = "lowercase")]`. */
export type Platform = "macos" | "windows";

/** `crates/core/src/installation.rs`: kebab-case. Declaration order = Ord. */
export type InstallationStatus =
  | "unsupported"
  | "permission-required"
  | "available";

/** `crates/core/src/installation.rs` `AgentInstallation`. */
export interface AgentInstallation {
  id: string;
  provider: ProviderId;
  platform: Platform;
  version: string | null;
  data_roots: string[];
  status: InstallationStatus;
}

// --- Capabilities ---

/** `crates/core/src/capability.rs`: kebab-case. Declaration order = Ord. */
export type CapabilityStatus =
  | "unsupported"
  | "permission-required"
  | "degraded"
  | "read-only"
  | "supported";

/** `crates/core/src/capability.rs`: kebab-case. */
export type CapabilityTopic =
  | "sessions"
  | "projects"
  | "archive"
  | "restore-archive"
  | "cache"
  | "logs"
  | "cleanup";

/**
 * `crates/core/src/capability.rs` `AgentCapabilities` is
 * `#[serde(transparent)]` over `BTreeMap<CapabilityTopic, CapabilityStatus>`,
 * so it serializes as a flat object keyed by topic. Missing topics imply
 * `Unsupported`; we keep this as a `Partial<...>` shape.
 */
export type AgentCapabilities = Partial<Record<CapabilityTopic, CapabilityStatus>>;

// --- Inspection ---

/** `crates/provider-api/src/lib.rs` `ProviderInspection`. */
export interface ProviderInspection {
  version: string | null;
  schema_versions: Record<string, string>;
  journal_modes: Record<string, string>;
  agent_running: boolean;
  readable_roots: Record<string, boolean>;
  unknown_structures: string[];
  problems: Record<string, ScanProblem>;
}

/** `crates/application/src/lib.rs` `DoctorReport`. */
export interface DoctorReport {
  installation: AgentInstallation;
  inspection: ProviderInspection;
  capabilities: AgentCapabilities;
}

// --- Resources ---

/** `crates/core/src/resource.rs`: kebab-case. */
export type ResourceKind =
  | "session"
  | "workspace"
  | "project-metadata"
  | "subagent"
  | "generated-file"
  | "cache"
  | "log"
  | "checkpoint"
  | "database"
  | "unknown";

/** `crates/core/src/resource.rs`: lowercase. */
export type Ownership = "exclusive" | "shared" | "unknown";

/** `crates/core/src/resource.rs`: lowercase. */
export type ManagedBy = "agent" | "agenttidy" | "user";

/**
 * `crates/core/src/resource.rs`: `#[serde(tag = "type", rename_all = "kebab-case")]`.
 * Tagged enum; each variant carries the fields shown.
 */
export type ResourceLocator =
  | { type: "file"; path: string }
  | { type: "dir"; path: string }
  | { type: "file-set"; paths: string[] }
  | {
      type: "database-record";
      database: string;
      record_id: string;
    };

/** `crates/core/src/resource.rs`. */
export interface ResourceRef {
  id: string;
  kind: ResourceKind;
}

/** `crates/core/src/resource.rs`. */
export interface Resource {
  id: string;
  provider: ProviderId;
  installation_id: string;
  kind: ResourceKind;
  locator: ResourceLocator;
  ownership: Ownership;
  managed_by: ManagedBy;
  size: SizeInfo;
  session_id: string | null;
  project: ProjectRef | null;
  created_at: number | null;
  updated_at: number | null;
  dependencies: ResourceRef[];
  metadata: Record<string, unknown>;
}

// --- Sessions ---

/** `crates/core/src/session.rs` `ProjectRef`. */
export interface ProjectRef {
  cwd: string | null;
  display_name: string | null;
}

/**
 * `crates/core/src/session.rs`: `#[serde(tag = "state", rename_all = "lowercase")]`.
 * Archived carries an optional `archived_at` (epoch ms).
 */
export type SessionLifecycle =
  | { state: "active" }
  | { state: "inactive" }
  | { state: "unknown" }
  | { state: "archived"; archived_at: number | null };

/** `crates/core/src/session.rs` `Session`. */
export interface Session {
  id: string;
  provider: ProviderId;
  installation_id: string;
  title: string | null;
  project: ProjectRef | null;
  created_at: number | null;
  updated_at: number | null;
  lifecycle: SessionLifecycle;
  size: SizeInfo;
  resource_refs: ResourceRef[];
  metadata: Record<string, unknown>;
}

// --- Sizes ---

/** `crates/core/src/size.rs`: lowercase. */
export type SizeConfidence = "exact" | "estimated" | "unknown";

/** `crates/core/src/size.rs`. */
export interface SizeInfo {
  logical_bytes: number;
  allocated_bytes: number | null;
  exclusive_bytes: number | null;
  shared_bytes: number | null;
  reclaimable_bytes: number | null;
  confidence: SizeConfidence;
}

// --- Snapshot ---

/**
 * `crates/core/src/snapshot.rs`: `#[serde(tag = "severity", rename_all = "lowercase")]`.
 */
export type ScanProblem =
  | { severity: "warning"; message: string }
  | {
      severity: "error";
      message: string;
      path: string | null;
    };

/** `crates/core/src/snapshot.rs`. */
export interface ScanOptions {
  include_unknown: boolean;
}

/** `crates/core/src/snapshot.rs` `AgentSnapshot`. */
export interface AgentSnapshot {
  installation: AgentInstallation;
  sessions: Session[];
  resources: Resource[];
  problems: Record<string, ScanProblem>;
  completed_at: number;
}
// ---------------------------------------------------------------------------
// Phase 6 cleanup model — crates/core/src/cleanup.rs (Start.md §7, §11, §12).
// ---------------------------------------------------------------------------

/** `crates/core/src/cleanup.rs` `RiskLevel`: kebab-case. §7 vocabulary. */
export type RiskLevel = "low-risk" | "review-required" | "blocked";

/** `crates/core/src/cleanup.rs`: kebab-case (§9.5 unit kinds). */
export type CleanupUnitKind = "file-tree" | "file-set" | "provider-operation";

/** `crates/core/src/cleanup.rs`: kebab-case. `trash` ⇒ OS Recycle Bin / Finder Trash. */
export type CleanupAction = "trash" | "provider-operation";

/** `crates/core/src/cleanup.rs`: kebab-case (§12.3 per-item outcomes). */
export type CleanupOutcome = "pending" | "removed" | "skipped" | "failed";

/** `crates/core/src/cleanup.rs`: kebab-case — one per §12.2 invalidation trigger. */
export type CleanupPreconditionKind =
  | "path-exists"
  | "path-unchanged"
  | "identity-unchanged"
  | "size-unchanged"
  | "mtime-unchanged"
  | "no-agent-running"
  | "session-inactive"
  | "session-still-exclusive"
  | "schema-unchanged"
  | "writer-lock-absent"
  | "filesystem-eligible"
  | "no-symlink-outside-root";

/** `crates/core/src/cleanup.rs` `FileIdentity` (volume + index / dev + ino). */
export interface FileIdentity {
  volume: number;
  index: number;
}

/** `crates/core/src/cleanup.rs` `ResourceFingerprint` — §12.2 compare set. */
export interface ResourceFingerprint {
  path: string;
  identity: FileIdentity;
  size_bytes: number;
  mtime_ms: number;
}

/** `crates/core/src/cleanup.rs` `CleanupPrecondition`. */
export interface CleanupPrecondition {
  kind: CleanupPreconditionKind;
  description: string;
}

/**
 * `crates/core/src/cleanup.rs` `CleanupLocator` — serde-tagged on `kind`,
 * kebab-case. `file-set` carries *all* member paths (§9.5 atomicity):
 * e.g. a Claude Code transcript plus its same-named side directory.
 */
export type CleanupLocator =
  | { kind: "file"; path: string }
  | { kind: "dir"; path: string }
  | { kind: "file-set"; paths: string[] }
  | { kind: "db-row"; database: string; record_id: string };

/** `crates/core/src/cleanup.rs` `CleanupItem` — one plan row. */
/** Preview evidence is returned unchanged over IPC; it contains no message bodies. */
export interface CleanupEvidence {
  provider_version: string | null;
  schema_versions: Record<string, string>;
  journal_modes: Record<string, string>;
  canonical_paths: Record<string, string>;
  members: ResourceFingerprint[];
}

export interface CleanupItem {
  evidence: CleanupEvidence;
  unit_id: string;
  installation_id: string;
  provider: ProviderId;
  fingerprint: ResourceFingerprint;
  /** Denormalized from the unit — the executor moves every member path. */
  locator: CleanupLocator;
  /** Logical estimate for review; never a measurement of immediate disk reclamation. */
  estimated_reclaimable_bytes: number;
  action: CleanupAction;
  risk: RiskLevel;
  reasons: string[];
  preconditions: CleanupPrecondition[];
}

/** `crates/core/src/cleanup.rs` `RiskSummary` — §6.1 byte totals per bucket. */
export interface RiskSummary {
  low_risk: number;
  low_risk_bytes: number;
  review_required: number;
  review_required_bytes: number;
  blocked: number;
  blocked_bytes: number;
}

/** `crates/core/src/cleanup.rs` `CleanupPlan` — held by the GUI between confirmations. */
export interface CleanupPlan {
  id: string;
  scan_id: string;
  created_at: number;
  expires_at: number | null;
  items: CleanupItem[];
  total_reclaimable_bytes: number;
  risk_summary: RiskSummary;
  fingerprint: string;
}

/** `crates/core/src/cleanup.rs` `CleanupItemOutcome` (§12.3). */
export interface CleanupItemOutcome {
  unit_id: string;
  outcome: CleanupOutcome;
  reclaimed_bytes: number;
  reason: string | null;
}

/** `crates/core/src/cleanup.rs` `CleanupRevalidationOutcome` (§12.2). */
export interface CleanupRevalidationOutcome {
  unit_id: string;
  outcome: CleanupOutcome;
  failed_precondition: CleanupPreconditionKind | null;
  reason: string | null;
}

/** `apps/desktop/src-tauri/src/commands.rs` `JsonEnvelope<T>` — `agenttidy.gui.v1`. */
export interface JsonEnvelope<T> {
  schema_version: "agenttidy.gui.v1";
  command: string;
  mode: string;
  data: T;
}
