// Cleanup request lifecycle (Start.md §6.3 / §12). Backend admission remains
// independent; these UI gates prevent stale responses and repeated execution.
import { useCallback, useEffect, useRef, useState } from "react";
import { cleanupExecute, cleanupPreview, cleanupRevalidate } from "./api";
import { canExecute, CleanupRequests, selectableIds, type CleanupPhase } from "./cleanup-flow";
import type { CleanupItemOutcome, CleanupPlan, CleanupRevalidationOutcome } from "./types";
const errorText = (reason: unknown): string => reason instanceof Error ? reason.message : String(reason);

export interface CleanupState {
  plan: CleanupPlan | null; selectedIds: ReadonlySet<string>;
  revalidation: CleanupRevalidationOutcome[]; outcomes: CleanupItemOutcome[] | null;
  loading: boolean; phase: CleanupPhase; error: string | null;
  revalidated: boolean; executionAttempted: boolean;
  refreshPreview: () => void; toggle: (unitId: string) => void;
  revalidate: () => void; execute: () => void;
}

export function useCleanupState(enabled: boolean): CleanupState {
  const [initialized, setInitialized] = useState(false);
  const [plan, setPlan] = useState<CleanupPlan | null>(null);
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(() => new Set());
  const [revalidation, setRevalidation] = useState<CleanupRevalidationOutcome[]>([]);
  const [outcomes, setOutcomes] = useState<CleanupItemOutcome[] | null>(null);
  const [phase, setPhase] = useState<CleanupPhase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [revalidated, setRevalidated] = useState(false);
  const [executionAttempted, setExecutionAttempted] = useState(false);
  const spent = useRef(false);
  const requests = useRef(new CleanupRequests());
  const finish = useCallback((token: number) => {
    if (!requests.current.current(token)) return;
    requests.current.finish(token); setPhase("idle");
  }, []);

  const refreshPreview = useCallback(() => {
    const token = requests.current.begin("preview");
    if (token === null) return;
    setPhase("preview"); setError(null); setRevalidated(false);
    // A failed refresh must not leave the former plan executable.
    setPlan(null); setSelectedIds(new Set()); setOutcomes(null); setRevalidation([]);
    spent.current = true; setExecutionAttempted(true);
    cleanupPreview().then(nextPlan => {
      if (!requests.current.current(token)) return;
      setPlan(nextPlan);
      setSelectedIds(new Set(nextPlan.items.filter(item => item.risk === "low-risk").map(item => item.unit_id)));
      spent.current = false; setExecutionAttempted(false);
    }).catch(reason => { if (requests.current.current(token)) setError(errorText(reason)); })
      .finally(() => finish(token));
  }, [finish]);

  // Browsing starts with one read-only scan. Generate a plan on the first
  // visit to Review; leaving that tab must not cancel an in-flight execution.
  useEffect(() => {
    if (enabled && !initialized) { setInitialized(true); refreshPreview(); }
  }, [enabled, initialized, refreshPreview]);
  useEffect(() => () => requests.current.cancel(), []);

  const toggle = useCallback((unitId: string) => {
    if (requests.current.busy() || spent.current) return;
    if (!selectableIds(plan, new Set([unitId]), revalidation).has(unitId)) return;
    setRevalidated(false);
    setSelectedIds(current => {
      const next = new Set(current);
      if (next.has(unitId)) next.delete(unitId); else next.add(unitId);
      return next;
    });
  }, [plan, revalidation]);

  const revalidate = useCallback(() => {
    if (!plan || spent.current || selectedIds.size === 0) return;
    const token = requests.current.begin("revalidate");
    if (token === null) return;
    setPhase("revalidate"); setError(null); setRevalidated(false);
    cleanupRevalidate(plan).then(results => {
      if (!requests.current.current(token)) return;
      setRevalidation(results); setRevalidated(true);
      setSelectedIds(current => selectableIds(plan, current, results));
    }).catch(reason => { if (requests.current.current(token)) setError(errorText(reason)); })
      .finally(() => finish(token));
  }, [plan, selectedIds.size, finish]);

  const execute = useCallback(() => {
    if (!canExecute(plan, selectedIds, revalidation, revalidated, spent.current)) return;
    const token = requests.current.begin("execute");
    if (token === null || !plan) return;
    spent.current = true; setExecutionAttempted(true); setPhase("execute");
    setError(null); setRevalidated(false);
    const ids = [...selectedIds];
    setSelectedIds(new Set());
    cleanupExecute(plan, plan.fingerprint, ids).then(results => {
      if (requests.current.current(token)) setOutcomes(results);
    }).catch(reason => {
      // A lost IPC response may follow a partial mutation. A fresh preview,
      // never a silent retry with the old CONFIRM, is required after any attempt.
      if (requests.current.current(token)) setError(errorText(reason));
    }).finally(() => finish(token));
  }, [plan, selectedIds, revalidation, revalidated, finish]);

  return { plan, selectedIds, revalidation, outcomes, loading: phase !== "idle", phase,
    error, revalidated, executionAttempted, refreshPreview, toggle, revalidate, execute };
}
