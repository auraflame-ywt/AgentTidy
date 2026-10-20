// Sessions view — Start.md §6.2.
//
// Flat table of every recognisable Session across all installations.
// Columns are the unified fields §6.2 calls out: Agent (provider +
// installation), stable session id, project cwd, lifecycle (with
// archived_at where present), size with confidence, and last-activity
// timestamp. Empty state matches the CLI's "No recognizable sessions
// found." branch. i18n: headers, lifecycle labels and the empty state go
// through `useI18n().t`; timestamps are locale-formatted.

import { useState } from "react";
import { DEFAULT_SESSION_QUERY, querySessions, sessionRows, type SessionQuery } from "../session-query";
import { formatDateTime, humanBytes } from "../format";
import { useI18n } from "../i18n";
import SessionDetail from "./SessionDetail";
import { pathName, sessionTitle } from "../presentation";
import type { AgentSnapshot, CleanupPlan, Session, SizeConfidence } from "../types";

type Translate = ReturnType<typeof useI18n>["t"];

function lifecycleLabel(session: Session, t: Translate): {
  label: string;
  detail?: number;
} {
  switch (session.lifecycle.state) {
    case "active":
      return { label: t("lifecycle.active") };
    case "inactive":
      return { label: t("lifecycle.inactive") };
    case "unknown":
      return { label: t("lifecycle.unknown") };
    case "archived":
      return {
        label: t("lifecycle.archived"),
        detail: session.lifecycle.archived_at ?? undefined,
      };
  }
}

function lifecycleClass(state: string): string {
  switch (state) {
    case "active":
      return "bg-emerald-100 text-emerald-800";
    case "inactive":
      return "bg-sky-100 text-sky-800";
    case "archived":
      return "bg-violet-100 text-violet-800";
    case "unknown":
      return "bg-neutral-200 text-neutral-700";
    default:
      return "bg-neutral-200 text-neutral-700";
  }
}

function confidenceLabel(confidence: SizeConfidence, t: Translate): string {
  switch (confidence) {
    case "exact":
      return t("confidence.exact");
    case "estimated":
      return t("confidence.estimated");
    case "unknown":
      return t("confidence.unknown");
  }
}

interface SessionsProps {
  snapshots: AgentSnapshot[];
  plan: CleanupPlan | null;
  query: SessionQuery;
  onQueryChange: (query: SessionQuery) => void;
  onReview: () => void;
}

export default function Sessions({ snapshots, plan, query, onQueryChange, onReview }: SessionsProps) {
  const { t, lang } = useI18n();
  const [page, setPage] = useState(0);
  const [opened, setOpened] = useState<{ installation: string; id: string } | null>(null);
  const detailSnapshot = snapshots.find(snapshot => snapshot.installation.id === opened?.installation);
  const detailSession = detailSnapshot?.sessions.find(session => session.id === opened?.id);
  const allRows = sessionRows(snapshots);
  const rows = querySessions(allRows, query, Date.now());
  const pageSize = 50;
  const currentPage = Math.min(page, Math.max(0, Math.ceil(rows.length / pageSize) - 1));
  const setFilter = (next: Partial<SessionQuery>) => { onQueryChange({ ...query, ...next }); setPage(0); };
  if (allRows.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-neutral-300 bg-white p-10 text-center">
        <h2 className="text-lg font-medium text-neutral-700">
          {t("sessions.empty.title")}
        </h2>
        <p className="mt-2 text-sm text-neutral-500">
          {t("sessions.empty.body")}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div><h2 className="text-xl font-semibold">{t("tab.sessions")}</h2><p className="mt-1 text-sm text-neutral-500">{t("sessions.intro")}</p></div>
      <div className="flex flex-wrap items-end gap-3 rounded-lg border border-neutral-200 bg-white p-4">
        <label className="min-w-52 flex-1 text-xs text-neutral-600">{t("sessions.search")}
          <input type="search" value={query.text} onChange={event => setFilter({ text: event.target.value })}
            placeholder={t("sessions.searchHint")} className="mt-1 block w-full rounded-md border border-neutral-300 px-3 py-2 text-sm" />
        </label>
        <label className="text-xs text-neutral-600">{t("sessions.filter.provider")}
          <select value={query.provider} onChange={event => setFilter({ provider: event.target.value as SessionQuery["provider"] })}
            className="mt-1 block rounded-md border border-neutral-300 px-2 py-2 text-sm">
            <option value="all">{t("sessions.filter.all")}</option>
            <option value="claude-code">{t("provider.claudeCode")}</option>
            <option value="codex">{t("provider.codex")}</option>
            <option value="workbuddy">{t("provider.workbuddy")}</option>
          </select>
        </label>
        <label className="text-xs text-neutral-600">{t("sessions.col.lifecycle")}
          <select value={query.lifecycle} onChange={event => setFilter({ lifecycle: event.target.value as SessionQuery["lifecycle"] })}
            className="mt-1 block rounded-md border border-neutral-300 px-2 py-2 text-sm">
            <option value="all">{t("sessions.filter.all")}</option>
            {(["active", "inactive", "archived", "unknown"] as const).map(state =>
              <option key={state} value={state}>{t(`lifecycle.${state}`)}</option>)}
          </select>
        </label>
        <label className="text-xs text-neutral-600">{t("sessions.filter.period")}
          <select value={query.period} onChange={event => setFilter({ period: event.target.value as SessionQuery["period"] })}
            className="mt-1 block rounded-md border border-neutral-300 px-2 py-2 text-sm">
            {(["all", "week", "month", "older"] as const).map(period =>
              <option key={period} value={period}>{t(`sessions.period.${period}`)}</option>)}
          </select>
        </label>
        <label className="text-xs text-neutral-600">{t("sessions.sort")}
          <select value={query.order} onChange={event => setFilter({ order: event.target.value as SessionQuery["order"] })}
            className="mt-1 block rounded-md border border-neutral-300 px-2 py-2 text-sm">
            {(["newest", "oldest", "largest", "smallest"] as const).map(order =>
              <option key={order} value={order}>{t(`sessions.sort.${order}`)}</option>)}
          </select>
        </label>
        <button type="button" onClick={() => { onQueryChange(DEFAULT_SESSION_QUERY); setPage(0); }}
          className="rounded-md border border-neutral-300 px-3 py-2 text-sm">{t("sessions.reset")}</button>
      </div>
      <p className="text-xs text-neutral-500" aria-live="polite">{t("sessions.count", { shown: rows.length, total: allRows.length })}</p>
      {rows.length === 0 ? <p className="rounded-lg border border-dashed border-neutral-300 p-8 text-center text-sm text-neutral-500">{t("sessions.noMatches")}</p> :
      <div className="overflow-x-auto rounded-lg border border-neutral-200 bg-white">
      <table className="w-full text-left text-sm">
        <thead className="border-b border-neutral-200 bg-neutral-50 text-xs uppercase tracking-wide text-neutral-500">
          <tr>
            <th className="px-4 py-2">{t("sessions.col.installation")}</th>
            <th className="px-4 py-2">{t("sessions.col.session")}</th>
            <th className="px-4 py-2">{t("sessions.col.project")}</th>
            <th className="px-4 py-2">{t("sessions.col.lifecycle")}</th>
            <th className="px-4 py-2">{t("sessions.col.size")}</th>
            <th className="px-4 py-2">{t("sessions.col.updated")}</th>
          </tr>
        </thead>
        <tbody>
          {rows.slice(currentPage * pageSize, (currentPage + 1) * pageSize).map(({ installation, session }) => {
            const lifecycle = lifecycleLabel(session, t);
            return (
              <tr
                key={`${installation}:${session.id}`}
                className="border-b border-neutral-100 last:border-b-0"
              >
                <td className="px-4 py-2 font-mono text-xs text-neutral-700">
                  {session.provider === "claude-code" ? t("provider.claudeCode") : session.provider === "codex" ? t("provider.codex") : t("provider.workbuddy")}
                </td>
                <td className="px-4 py-2 font-mono text-xs text-neutral-900">
                  <button type="button" onClick={() => setOpened({ installation, id: session.id })}
                    className="max-w-64 truncate text-left font-sans text-sm font-medium text-neutral-900 underline decoration-neutral-200 underline-offset-4 hover:decoration-emerald-500">
                    {sessionTitle(session)}
                  </button>
                </td>
                <td className="px-4 py-2 text-xs text-neutral-700">
                  <span title={session.project?.cwd ?? undefined}>{session.project?.display_name || (session.project?.cwd ? pathName(session.project.cwd) : "—")}</span>
                </td>
                <td className="px-4 py-2">
                  <span
                    className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${lifecycleClass(session.lifecycle.state)}`}
                  >
                    {lifecycle.label}
                  </span>
                  {lifecycle.detail !== undefined ? (
                    <span className="ml-2 text-[11px] text-neutral-500">
                      {formatDateTime(lifecycle.detail, lang)}
                    </span>
                  ) : null}
                </td>
                <td className="px-4 py-2 text-xs text-neutral-700">
                  {humanBytes(session.size.logical_bytes)}
                  <span className="ml-1 text-[10px] text-neutral-500">
                    ({confidenceLabel(session.size.confidence, t)})
                  </span>
                </td>
                <td className="px-4 py-2 text-xs text-neutral-700">
                  {session.updated_at != null
                    ? formatDateTime(session.updated_at, lang)
                    : "—"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      </div>}
      {detailSnapshot && detailSession ? <SessionDetail snapshot={detailSnapshot} session={detailSession} plan={plan}
        onClose={() => setOpened(null)} onReview={() => { setOpened(null); onReview(); }} /> : null}
      {rows.length > pageSize ? <div className="flex items-center justify-end gap-3 text-xs text-neutral-600">
        <button type="button" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}
          className="rounded-md border border-neutral-300 px-3 py-2 disabled:opacity-40">{t("sessions.previous")}</button>
        <span>{t("sessions.page", { current: currentPage + 1, total: Math.ceil(rows.length / pageSize) })}</span>
        <button type="button" disabled={(currentPage + 1) * pageSize >= rows.length} onClick={() => setPage(currentPage + 1)}
          className="rounded-md border border-neutral-300 px-3 py-2 disabled:opacity-40">{t("sessions.next")}</button>
      </div> : null}
    </div>
  );
}