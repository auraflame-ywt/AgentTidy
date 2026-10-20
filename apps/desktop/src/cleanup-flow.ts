// Cleanup UI admission and request ordering (Start.md §6.3 / §12.2).
// These gates prevent duplicate IPC calls and late responses from restoring
// a stale selection; backend revalidation remains independently mandatory.
import type { CleanupPlan, CleanupRevalidationOutcome } from "./types";

export type CleanupPhase = "idle" | "preview" | "revalidate" | "execute";
export class CleanupRequests {
  private revision = 0;
  private active: CleanupPhase = "idle";
  begin(phase: Exclude<CleanupPhase, "idle">): number | null {
    if (this.active !== "idle") return null;
    this.active = phase;
    return ++this.revision;
  }
  current(token: number): boolean { return token === this.revision && this.active !== "idle"; }
  finish(token: number): void { if (this.current(token)) this.active = "idle"; }
  cancel(): void { ++this.revision; this.active = "idle"; }
  busy(): boolean { return this.active !== "idle"; }
}

export function planExpired(plan: CleanupPlan | null, now = Date.now()): boolean {
  return plan?.expires_at != null && now > plan.expires_at;
}

export function selectableIds(plan: CleanupPlan | null, ids: ReadonlySet<string>,
  revalidation: CleanupRevalidationOutcome[]): Set<string> {
  const stale = new Set(revalidation.filter(item => item.outcome !== "pending").map(item => item.unit_id));
  return new Set(plan?.items.filter(item => ids.has(item.unit_id)
    && item.risk !== "blocked" && !stale.has(item.unit_id)).map(item => item.unit_id) ?? []);
}

export function canExecute(plan: CleanupPlan | null, ids: ReadonlySet<string>,
  revalidation: CleanupRevalidationOutcome[], revalidated: boolean, spent: boolean, now = Date.now()): boolean {
  return !!plan && !planExpired(plan, now) && revalidated && !spent && ids.size > 0
    && selectableIds(plan, ids, revalidation).size === ids.size;
}
