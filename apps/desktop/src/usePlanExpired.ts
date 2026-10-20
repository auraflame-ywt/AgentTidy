// A waiting Mac confirmation must expire without requiring another click
// or refresh (§12.2). Click handlers still check the wall clock themselves.
import { useEffect, useState } from "react";
import { planExpired } from "./cleanup-flow";
import type { CleanupPlan } from "./types";

export function usePlanExpired(plan: CleanupPlan | null): boolean {
  const [now, setNow] = useState(Date.now);
  const expiry = plan?.expires_at;
  useEffect(() => {
    setNow(Date.now());
    if (expiry == null) return;
    const timer = window.setTimeout(() => setNow(Date.now()),
      Math.max(0, Math.min(expiry - Date.now() + 1, 2_147_483_647)));
    return () => window.clearTimeout(timer);
  }, [expiry]);
  return planExpired(plan, Math.max(now, Date.now()));
}
