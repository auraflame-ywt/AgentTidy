// Session identity, footprint and cleanup availability (Start.md §6.2).
// No transcript bodies are requested; all fields come from existing metadata.
import { providerKeys } from "../provider-catalog";
import DetailPanel from "../components/DetailPanel";
import { useI18n } from "../i18n";
import { formatDateTime, humanBytes } from "../format";
import { resourcePaths, sessionCleanupItems, sessionResources, sessionTitle } from "../presentation";
import { usePlanExpired } from "../usePlanExpired";
import type { AgentSnapshot, CleanupPlan, Session } from "../types";

export default function SessionDetail({ snapshot, session, plan, onClose, onReview }: {
  snapshot: AgentSnapshot; session: Session; plan: CleanupPlan | null;
  onClose: () => void; onReview: () => void;
}) {
  const { t, lang, translateCleanupReason } = useI18n();
  const resources = sessionResources(snapshot, session);
  // A multi-root session has no single project cwd; preserve all index links.
  const workspaces = Array.isArray(session.metadata?.workspace_uris)
    ? session.metadata.workspace_uris.filter((uri): uri is string => typeof uri === "string") : [];
  const items = sessionCleanupItems(snapshot, session, plan);
  const date = session.updated_at ?? session.created_at;
  const expired = usePlanExpired(plan);
  const valid = !!plan && !expired;
  return <DetailPanel title={sessionTitle(session)} onClose={onClose}>
    <p className="text-sm text-neutral-500">{t("detail.metadataOnly")}</p>
    <dl className="grid grid-cols-2 gap-x-5 gap-y-4 text-sm">
      <div><dt className="text-xs text-neutral-500">{t("sessions.filter.provider")}</dt><dd className="mt-1">{t(providerKeys[session.provider])}</dd></div>
      <div><dt className="text-xs text-neutral-500">{t("sessions.col.lifecycle")}</dt><dd className="mt-1">{t(`lifecycle.${session.lifecycle.state}`)}</dd></div>
      <div><dt className="text-xs text-neutral-500">{t("sessions.col.size")}</dt><dd className="mt-1 text-lg font-semibold">{humanBytes(session.size.logical_bytes)}</dd></div>
      <div><dt className="text-xs text-neutral-500">{t("sessions.col.updated")}</dt><dd className="mt-1">{date == null ? t("detail.unknown") : formatDateTime(date, lang)}</dd></div>
      <div className="col-span-2"><dt className="text-xs text-neutral-500">{t("sessions.col.project")}</dt><dd className="mt-1 break-all">{session.project?.cwd ?? t("detail.unknown")}</dd></div>
    </dl>
    {workspaces.length > 1 ? <section><h3 className="text-sm font-semibold">{t("detail.workspaces", { count: workspaces.length })}</h3>
      <ul className="mt-2 space-y-2 text-xs text-neutral-600">{workspaces.map((uri, index) => <li key={index} className="break-all">{uri}</li>)}</ul>
    </section> : null}
    <section className="rounded-lg border border-neutral-200 bg-neutral-50 p-4">
      <h3 className="text-sm font-semibold">{t("detail.cleanup")}</h3>
      <p className="mt-2 text-sm leading-6 text-neutral-600">{!valid ? t("detail.notEvaluated") : items.length === 0 ? t("detail.noUnit") : t("detail.reviewRequired")}</p>
      {valid ? items.map(item => <div key={item.unit_id} className="mt-3 border-t border-neutral-200 pt-3 text-sm">
        <p className="font-medium">{t(item.risk === "blocked" ? "risk.blocked" : item.risk === "low-risk" ? "risk.lowRisk" : "risk.reviewRequired")}</p>
        <ul className="mt-1 space-y-2 text-neutral-600">{item.reasons.map((reason, index) => <li key={index}>{translateCleanupReason(reason)}</li>)}</ul>
      </div>) : null}
      <button type="button" onClick={onReview} className="mt-3 rounded-md bg-neutral-900 px-3 py-2 text-sm font-medium text-white">{t("detail.openReview")}</button>
    </section>
    <section><h3 className="text-sm font-semibold">{t("detail.resources", { count: resources.length })}</h3>
      <p className="mt-1 text-xs text-neutral-500">{t("detail.logicalSize")}</p>
      <ul className="mt-3 space-y-3">{resources.map(resource => <li key={resource.id} className="rounded-lg border border-neutral-200 p-3">
        <div className="flex justify-between gap-3 text-xs"><span>{t(`detail.kind.${resource.kind}`)} · {t(`detail.ownership.${resource.ownership}`)}</span><span>{humanBytes(resource.size.logical_bytes)} · {t(`confidence.${resource.size.confidence}`)}</span></div>
        {resourcePaths(resource).map(path => <p key={path} className="mt-2 break-all font-mono text-xs leading-5 text-neutral-600">{path}</p>)}
      </li>)}</ul>
      {resources.length === 0 ? <p className="mt-2 text-sm text-neutral-500">{t("detail.noResources")}</p> : null}
    </section>
    <details className="text-xs text-neutral-500"><summary className="cursor-pointer">{t("detail.technical")}</summary><p className="mt-2 break-all font-mono">{session.id}</p></details>
  </DetailPanel>;
}
