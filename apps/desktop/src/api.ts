// Typed wrappers around the Tauri `invoke` IPC. The Rust side defines each
// command in `apps/desktop/src-tauri/src/commands.rs`; the strings here
// must match those `#[tauri::command]` names exactly. `tsc -b` checks these
// wrappers against the manually mirrored `./types`; it cannot detect a Rust
// field rename by itself. IPC shape changes require cross-boundary validation.

import { invoke } from "@tauri-apps/api/core";
import type {
  AgentInstallation,
  AgentSnapshot,
  DoctorReport,
} from "./types";

export function doctor(): Promise<DoctorReport[]> {
  return invoke<DoctorReport[]>("doctor");
}

export function scan(): Promise<AgentSnapshot[]> {
  return invoke<AgentSnapshot[]>("scan");
}

export function detect(): Promise<AgentInstallation[]> {
  return invoke<AgentInstallation[]>("detect");
}
import type {
  CleanupItemOutcome,
  CleanupPlan,
  CleanupRevalidationOutcome,
  JsonEnvelope,
} from "./types";

/**
 * Phase 6 cleanup IPC — three commands map to the three steps of the
 * two-confirmation flow in `docs/safety/workspace-cleanup.md`. Each
 * response carries the `agenttidy.gui.v1` envelope; we unwrap `data`
 * here so call sites only see the payload.
 */
async function invokeData<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const envelope = await invoke<JsonEnvelope<T>>(command, args);
  if (envelope.schema_version !== "agenttidy.gui.v1") {
    throw new Error(
      `unsupported gui schema ${envelope.schema_version} for ${command}`,
    );
  }
  return envelope.data;
}

/** Step 1: build the typed cleanup plan (read-only). */
export function cleanupPreview(): Promise<CleanupPlan> {
  return invokeData<CleanupPlan>("cleanup_preview");
}

/** Step 2: revalidate between confirmations (§12.2). */
export function cleanupRevalidate(
  plan: CleanupPlan,
): Promise<CleanupRevalidationOutcome[]> {
  return invokeData<CleanupRevalidationOutcome[]>("cleanup_revalidate", {
    plan,
  });
}

/** Step 3: execute after the second confirmation (§12.1). */
export function cleanupExecute(
  plan: CleanupPlan,
  fingerprint: string,
  selectedUnitIds: string[],
): Promise<CleanupItemOutcome[]> {
  return invokeData<CleanupItemOutcome[]>("cleanup_execute", {
    plan,
    fingerprint,
    selectedUnitIds,
  });
}
