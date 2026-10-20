// Read-only page projections (Start.md §6 / §9). These helpers join verified
// snapshot facts for display; only the backend plan determines cleanup risk.
import type { AgentSnapshot, CleanupItem, CleanupPlan, DoctorReport, Resource, Session } from "./types";

export function pathName(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;
}

export function sessionTitle(session: Session): string {
  const shortId = session.id.length > 8 ? `${session.id.slice(0, 8)}…` : session.id;
  const project = session.project?.display_name || (session.project?.cwd ? pathName(session.project.cwd) : null);
  return session.title?.trim() || [project, shortId].filter(Boolean).join(" · ");
}

export function sessionResources(snapshot: AgentSnapshot, session: Session): Resource[] {
  const ids = new Set(session.resource_refs.map(ref => ref.id));
  return snapshot.resources.filter(resource => ids.has(resource.id) && resource.session_id === session.id);
}

export function resourcePaths(resource: Resource): string[] {
  switch (resource.locator.type) {
    case "file": case "dir": return [resource.locator.path];
    case "file-set": return resource.locator.paths;
    case "database-record": return [resource.locator.database];
  }
}

// Never infer session ownership by splitting provider-generated unit IDs or
// by a shared DB path. Ambiguous joins deliberately retain the technical label.
export function cleanupSession(item: CleanupItem, snapshots: AgentSnapshot[]): Session | null {
  const snapshot = snapshots.find(value => value.installation.id === item.installation_id
    && value.installation.provider === item.provider);
  if (!snapshot) return null;
  const paths = new Set(item.evidence.members.map(member => member.path));
  // Inspect each resource once per candidate, rather than filtering the
  // entire resource list separately for every session in a large installation.
  const resources = snapshot.resources.filter(resource => resource.locator.type !== "database-record"
    && resourcePaths(resource).some(path => paths.has(path)));
  const sessions = snapshot.sessions.filter(session => resources.some(resource => resource.session_id === session.id
    && session.resource_refs.some(ref => ref.id === resource.id)));
  return sessions.length === 1 ? sessions[0] : null;
}

export function sessionCleanupItems(snapshot: AgentSnapshot, session: Session, plan: CleanupPlan | null): CleanupItem[] {
  return plan?.items.filter(item => cleanupSession(item, [snapshot])?.id === session.id) ?? [];
}

export function diagnosticCount(report: DoctorReport, snapshot?: AgentSnapshot): number {
  return new Set([...Object.keys(report.inspection.problems), ...Object.keys(snapshot?.problems ?? {})]).size;
}

export function storageSummary(snapshots: AgentSnapshot[]) {
  const resources = snapshots.flatMap(snapshot => snapshot.resources);
  return {
    bytes: resources.reduce((sum, resource) => sum + resource.size.logical_bytes, 0),
    sessions: snapshots.reduce((sum, snapshot) => sum + snapshot.sessions.length, 0),
    estimated: resources.some(resource => resource.size.confidence !== "exact"),
    // The oldest contributing scan is honest when installations finish at
    // different times. This is a snapshot, never a live disk-space meter.
    scannedAt: snapshots.length ? Math.min(...snapshots.map(snapshot => snapshot.completed_at)) : null,
  };
}

// A structural eligibility message is useful in details, but the primary
// explanation should describe the session's lifecycle and required decision.
export function primaryCleanupReason(item: CleanupItem): string {
  if (item.risk === "blocked") return item.reasons[0] ?? "";
  return item.reasons.find(reason => reason.startsWith("all §7.1 conditions hold"))
    ?? item.reasons.find(reason => reason.startsWith("session "))
    ?? item.reasons[0] ?? "";
}
