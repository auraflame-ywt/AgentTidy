// Read-only scan request lifecycle (Start.md §6.1). Revisions prevent late
// responses from overwriting a newer refresh or an unmounted view.
import { useCallback, useEffect, useRef, useState } from "react";
import { doctor, scan } from "./api";
import type { AgentSnapshot, DoctorReport } from "./types";
const errorText = (reason: unknown): string => reason instanceof Error ? reason.message : String(reason);

export interface ScanState {
  reports: DoctorReport[]; snapshots: AgentSnapshot[]; loading: boolean;
  error: string | null; refresh: () => void;
}
export function useScanState(): ScanState {
  const [reports, setReports] = useState<DoctorReport[]>([]);
  const [snapshots, setSnapshots] = useState<AgentSnapshot[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  const refresh = useCallback(() => {
    const token = ++revision.current;
    setLoading(true); setError(null);
    Promise.all([doctor(), scan()]).then(([nextReports, nextSnapshots]) => {
      if (revision.current !== token) return;
      setReports(nextReports); setSnapshots(nextSnapshots);
    }).catch(reason => { if (revision.current === token) setError(errorText(reason)); })
      .finally(() => { if (revision.current === token) setLoading(false); });
  }, []);
  useEffect(() => { refresh(); return () => { ++revision.current; }; }, [refresh]);
  return { reports, snapshots, loading, error, refresh };
}

