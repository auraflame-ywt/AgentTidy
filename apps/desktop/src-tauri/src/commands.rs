//! Tauri IPC commands — the desktop shell's only bridge to the
//! `agenttidy-application` API (architecture red line #1 in `Start.md` §22).
//!
//! Every command constructs a fresh `Application` because the registry is
//! stateless and cheap, and `anyhow` errors are surfaced to the frontend as
//! a single `String` (Tauri's command trait accepts `Result<T, String>`
//! directly via `String: Serialize` and the blanket `Into<InvokeError>`
//! impl). No business logic lives here; this file is a thin call-through.

use agenttidy_application::{Application, DoctorReport};
use agenttidy_core::{
    AgentInstallation, AgentSnapshot, CleanupItemOutcome, CleanupPlan, CleanupRevalidationOutcome,
    ScanOptions,
};

/// Read-only diagnostic facts per detected installation: capability state,
/// readable roots and any inspection warnings. Always succeeds on a stable
/// machine; errors surface permission problems or process-signal failures.
#[tauri::command]
pub async fn doctor() -> Result<Vec<DoctorReport>, String> {
    Application::new()
        .doctor()
        .await
        .map_err(|error| format!("{error:#}"))
}

/// Full read-only snapshot per available installation: sessions, resources,
/// scan problems and the per-run `completed_at` timestamp. The frontend
/// also uses this to render the Sessions and Diagnostics views.
#[tauri::command]
pub async fn scan() -> Result<Vec<AgentSnapshot>, String> {
    Application::new()
        .scan(&ScanOptions::default())
        .await
        .map_err(|error| format!("{error:#}"))
}

/// Bare detection: which providers are installed and which data roots are
/// readable. Used by the GUI for a faster first-paint before the heavier
/// `scan` completes — Phase 6 adds a workspace-candidates command
/// that consumes this directly.
#[tauri::command]
pub async fn detect() -> Result<Vec<AgentInstallation>, String> {
    Application::new()
        .detect()
        .await
        .map_err(|error| format!("{error:#}"))
}

// ---------------------------------------------------------------------------
// Phase 6: cleanup framework IPC. Mirrors the CLI's `agenttidy.cli.v1`
// envelope with `agenttidy.gui.v1` so schema drift is detectable on the
// wire. The plan itself round-trips as JSON (no server-side plan store) —
// the GUI holds it between the two confirmations and sends it back at
// execute time; the backend validates the fingerprint + revalidates every
// item before touching a single path.
// ---------------------------------------------------------------------------

/// Versioned response envelope for the Phase 6 cleanup commands. `mode`
/// records which step of the two-confirmation flow produced this payload.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct JsonEnvelope<T> {
    pub schema_version: &'static str,
    pub command: &'static str,
    pub mode: &'static str,
    pub data: T,
}

const GUI_SCHEMA_VERSION: &str = "agenttidy.gui.v1";

fn envelope<T>(command: &'static str, mode: &'static str, data: T) -> JsonEnvelope<T> {
    JsonEnvelope {
        schema_version: GUI_SCHEMA_VERSION,
        command,
        mode,
        data,
    }
}

/// Step 1 — build a read-only preview with metadata evidence (§12.1).
/// Execute independently rediscovers installations and re-proves each unit;
/// this command's Application instance is never retained across requests.
#[tauri::command]
pub async fn cleanup_preview() -> Result<JsonEnvelope<CleanupPlan>, String> {
    let application = Application::new();
    let snapshots = application
        .scan(&ScanOptions::default())
        .await
        .map_err(|error| format!("{error:#}"))?;
    let plan = application
        .cleanup_plan_from_snapshots(&snapshots)
        .await
        .map_err(|error| format!("{error:#}"))?;
    Ok(envelope("cleanup_preview", "preview", plan))
}

/// Step 2 — revalidate between the two confirmations (`Start.md` §12.2).
/// Reports, per item, which precondition (if any) went stale since the
/// plan was built; the GUI disables the final confirm for stale rows.
#[tauri::command]
pub async fn cleanup_revalidate(
    plan: CleanupPlan,
) -> Result<JsonEnvelope<Vec<CleanupRevalidationOutcome>>, String> {
    let outcomes = Application::new()
        .cleanup_revalidate(&plan)
        .await
        .map_err(|error| format!("{error:#}"))?;
    Ok(envelope("cleanup_revalidate", "revalidate", outcomes))
}

/// Step 3 — execute the selected items after the second confirmation.
/// `fingerprint` must equal `plan.fingerprint` or nothing is touched;
/// every executed item is audit-logged (`Start.md` §16.3).
#[tauri::command]
pub async fn cleanup_execute(
    plan: CleanupPlan,
    fingerprint: String,
    selected_unit_ids: Vec<String>,
) -> Result<JsonEnvelope<Vec<CleanupItemOutcome>>, String> {
    let outcomes = Application::new()
        .cleanup_execute(&plan, &fingerprint, &selected_unit_ids)
        .await
        .map_err(|error| format!("{error:#}"))?;
    Ok(envelope("cleanup_execute", "execute", outcomes))
}
