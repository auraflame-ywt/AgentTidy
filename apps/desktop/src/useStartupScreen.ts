// Startup visibility is independent of scans and cleanup (Start.md §6 / §12).
// A completed first scan exits with a brief transition; retries and errors never
// fake success, while manual dismissal keeps slow scans running in the workspace.
import { useEffect, useState } from "react";

export function useStartupScreen(loading: boolean, error: string | null) {
  const [dismissed, setDismissed] = useState(false);
  const [leaving, setLeaving] = useState(false);
  useEffect(() => {
    if (loading || error || dismissed) { setLeaving(false); return; }
    setLeaving(true);
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const timer = window.setTimeout(() => setDismissed(true), reduced ? 0 : 420);
    return () => window.clearTimeout(timer);
  }, [loading, error, dismissed]);
  return { dismissed, leaving, dismiss: () => setDismissed(true) };
}
