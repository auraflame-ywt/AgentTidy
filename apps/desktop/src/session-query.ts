// Read-only session browsing rules (Start.md §6.2). Pure helpers keep time,
// missing metadata and deterministic ordering testable outside the WebView.
import type { AgentSnapshot, ProviderId, Session } from "./types";

export interface SessionRow { installation: string; session: Session }
export type SessionPeriod = "all" | "week" | "month" | "older";
export type SessionOrder = "newest" | "oldest" | "largest" | "smallest";
export interface SessionQuery {
  text: string;
  provider: ProviderId | "all";
  lifecycle: Session["lifecycle"]["state"] | "all";
  period: SessionPeriod;
  order: SessionOrder;
}
export const DEFAULT_SESSION_QUERY: SessionQuery = {
  text: "", provider: "all", lifecycle: "all", period: "all", order: "newest",
};

export function sessionRows(snapshots: AgentSnapshot[]): SessionRow[] {
  return snapshots.flatMap(snapshot => snapshot.sessions.map(session => ({
    installation: snapshot.installation.id, session,
  })));
}

export function querySessions(rows: SessionRow[], query: SessionQuery, now: number): SessionRow[] {
  const text = query.text.trim().toLocaleLowerCase();
  const day = 24 * 60 * 60 * 1000;
  const result = rows.filter(({ installation, session }) => {
    if (query.provider !== "all" && session.provider !== query.provider) return false;
    if (query.lifecycle !== "all" && session.lifecycle.state !== query.lifecycle) return false;
    if (text && ![session.id, session.title, session.project?.cwd, installation]
      .some(value => value?.toLocaleLowerCase().includes(text))) return false;
    const activity = session.updated_at ?? session.created_at;
    // Unknown activity is never silently treated as ancient or as “now”.
    if (query.period !== "all" && activity == null) return false;
    if (query.period === "week" && now - activity! > 7 * day) return false;
    if (query.period === "month" && now - activity! > 30 * day) return false;
    if (query.period === "older" && now - activity! <= 30 * day) return false;
    return true;
  });
  return result.sort((a, b) => {
    const left = query.order === "largest" || query.order === "smallest"
      ? a.session.size.logical_bytes : a.session.updated_at ?? a.session.created_at;
    const right = query.order === "largest" || query.order === "smallest"
      ? b.session.size.logical_bytes : b.session.updated_at ?? b.session.created_at;
    // Null dates sort last in either direction; equal keys have a stable tie-break.
    if (left == null && right != null) return 1;
    if (left != null && right == null) return -1;
    const difference = (left ?? 0) - (right ?? 0);
    if (difference) return query.order === "newest" || query.order === "largest" ? -difference : difference;
    return `${a.installation}:${a.session.id}`.localeCompare(`${b.installation}:${b.session.id}`);
  });
}
