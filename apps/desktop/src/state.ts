// Single useState holder for the desktop's read-only scan results.
//
// On mount the hook fetches `doctor` and `scan` in parallel; an explicit
// `refresh` re-runs the same pair. An empty `data` array is a *valid*
// result (matches the CLI's "no supported agent installations detected"
// branch — the empty `agenttidy.cli.v1` envelope in
// `apps/cli/tests/golden/empty__*.json`). Errors are surfaced as a single
// string so the UI can render an error banner without a structured shape.

import { useCallback, useEffect, useState } from "react";
import { doctor, scan } from "./api";
import type { AgentSnapshot, DoctorReport } from "./types";

export interface ScanState {
  reports: DoctorReport[];
  snapshots: AgentSnapshot[];
  loading: boolean;
  error: string | null;
  refresh: () => void;
}

export function useScanState(): ScanState {
  const [reports, setReports] = useState<DoctorReport[]>([]);
  const [snapshots, setSnapshots] = useState<AgentSnapshot[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    setLoading(true);
    setError(null);
    Promise.all([doctor(), scan()])
      .then(([nextReports, nextSnapshots]) => {
        setReports(nextReports);
        setSnapshots(nextSnapshots);
      })
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : String(reason));
      })
      .finally(() => {
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { reports, snapshots, loading, error, refresh };
}
import { cleanupExecute, cleanupPreview, cleanupRevalidate } from "./api";
import type {
  CleanupItemOutcome,
  CleanupPlan,
  CleanupRevalidationOutcome,
} from "./types";

/**
 * State for the §6.3 Review & Tidy tab (Phase 6).
 *
 * Two-confirmation flow lives entirely on the frontend:
 *   1. mount / refreshPreview → `cleanup_preview` (build plan)
 *   2. user picks rows → revalidate() → `cleanup_revalidate` (between
 *      confirmations, shows which rows went stale — §12.2)
 *   3. user confirms in ReviewConfirm modal → execute() →
 *      `cleanup_execute` (fingerprint guard, audit-logged — §16.3)
 *
 * The plan round-trips through IPC; nothing is persisted (a tab switch
 * that loses state simply re-previews — Refresh re-fetches).
 */
export interface CleanupState {
  plan: CleanupPlan | null;
  selectedIds: ReadonlySet<string>;
  revalidation: CleanupRevalidationOutcome[];
  outcomes: CleanupItemOutcome[] | null;
  loading: boolean;
  error: string | null;
  /** Step 2 in flight / done — drives the confirm modal gate. */
  revalidated: boolean;
  refreshPreview: () => void;
  toggle: (unitId: string) => void;
  revalidate: () => void;
  execute: () => void;
}

export function useCleanupState(): CleanupState {
  const [plan, setPlan] = useState<CleanupPlan | null>(null);
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [revalidation, setRevalidation] = useState<CleanupRevalidationOutcome[]>([]);
  const [outcomes, setOutcomes] = useState<CleanupItemOutcome[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [revalidated, setRevalidated] = useState(false);

  const refreshPreview = useCallback(() => {
    setLoading(true);
    setError(null);
    setRevalidated(false);
    setOutcomes(null);
    setRevalidation([]);
    cleanupPreview()
      .then((nextPlan) => {
        setPlan(nextPlan);
        // §6.3 default selection: low-risk checked, review-required and
        // blocked unchecked (blocked rows are also non-selectable in the UI).
        setSelectedIds(
          new Set(
            nextPlan.items
              .filter((item) => item.risk === "low-risk")
              .map((item) => item.unit_id),
          ),
        );
      })
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : String(reason));
      })
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    refreshPreview();
  }, [refreshPreview]);

  const toggle = useCallback(
    (unitId: string) => {
      // §6.3: blocked is *never* selectable and a stale (changed) row
      // must not re-enter the selection between the two confirmations —
      // refused at the state layer, not only via the disabled checkbox.
      const item = plan?.items.find((entry) => entry.unit_id === unitId);
      if (!item || item.risk === "blocked") return;
      const stale = revalidation.some(
        (entry) => entry.unit_id === unitId && entry.outcome !== "pending",
      );
      if (stale) return;
      setRevalidated(false); // selection changed ⇒ re-run step 2
      setSelectedIds((current) => {
        const next = new Set(current);
        if (next.has(unitId)) next.delete(unitId);
        else next.add(unitId);
        return next;
      });
    },
    [plan, revalidation],
  );

  const revalidate = useCallback(() => {
    if (!plan) return;
    setError(null);
    cleanupRevalidate(plan)
      .then((results) => {
        setRevalidation(results);
        setRevalidated(true);
        // §12.2: units that changed since the plan drop out of the
        // selection — this is what the `review.revalidateChanged` copy
        // promises the user.
        const staleIds = new Set(
          results
            .filter((entry) => entry.outcome !== "pending")
            .map((entry) => entry.unit_id),
        );
        if (staleIds.size > 0) {
          setSelectedIds(
            (current) =>
              new Set([...current].filter((id) => !staleIds.has(id))),
          );
        }
      })
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : String(reason));
      });
  }, [plan]);

  const execute = useCallback(() => {
    if (!plan) return;
    setError(null);
    // Recompute the selection list so stale (skipped) rows are still sent —
    // the backend revalidates independently and reports per-item outcomes.
    cleanupExecute(plan, plan.fingerprint, [...selectedIds])
      .then((results) => {
        setOutcomes(results);
        // Post-execute (§12.3): the plan is spent — clear the selection
        // and the step-2 flag so the next run requires a fresh scan /
        // preview instead of silently re-executing the same plan.
        setSelectedIds(new Set());
        setRevalidated(false);
      })
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : String(reason));
      });
  }, [plan, selectedIds]);

  return {
    plan,
    selectedIds,
    revalidation,
    outcomes,
    loading,
    error,
    revalidated,
    refreshPreview,
    toggle,
    revalidate,
    execute,
  };
}
