// Current-attempt receipts and manual recovery guidance (Start.md §12.3 / §16.4).
// Display only submitted units; backend outcomes also include unselected skips.
// Original paths are recovery references, never claimed Trash destinations.
import { useI18n } from "../i18n";
import { cleanupSession, sessionTitle } from "../presentation";
import type { AgentSnapshot, CleanupItem, CleanupItemOutcome } from "../types";

export default function ExecutionResults({ items, outcomes, attemptedIds, snapshots }: {
  items: CleanupItem[]; outcomes: CleanupItemOutcome[] | null; attemptedIds: ReadonlySet<string>; snapshots: AgentSnapshot[];
}) {
  const { t, translateCleanupReason } = useI18n();
  // A rejected/lost IPC response is not evidence that nothing moved.
  // Keep submitted original locations visible for manual verification.
  const attempted: CleanupItemOutcome[] = outcomes?.filter(outcome => attemptedIds.has(outcome.unit_id))
    ?? items.filter(item => attemptedIds.has(item.unit_id)).map(item => ({ unit_id: item.unit_id, outcome: "pending", reclaimed_bytes: 0, reason: null }));
  const count = (state: string) => attempted.filter(outcome => outcome.outcome === state).length;
  const trashAttempt = items.some(item => attemptedIds.has(item.unit_id) && item.action === "trash");
  return <section aria-labelledby="execution-title" className="rounded-xl border border-neutral-200 bg-white p-5">
    <h3 id="execution-title" className="font-semibold">{t("results.title")}</h3>
    <p role="status" className="mt-2 text-sm text-neutral-600">{outcomes ? t("results.summary", { removed: count("removed"), skipped: count("skipped"), failed: count("failed") }) : t("results.unknown")}</p>
    <p className="mt-1 text-xs text-neutral-500">{t("results.scope")}</p>
    <ul className="mt-4 divide-y divide-neutral-100">{attempted.map(outcome => {
      const item = items.find(item => item.unit_id === outcome.unit_id);
      const session = item ? cleanupSession(item, snapshots) : null;
      const paths = item?.locator.kind === "file-set" ? item.locator.paths : item?.locator.kind === "db-row" ? [item.locator.database] : item ? [item.locator.path] : [];
      return <li key={outcome.unit_id} className="py-3">
        <div className="flex justify-between gap-3 text-sm"><span className="break-words font-medium">{session ? sessionTitle(session) : outcome.unit_id}</span>
          <span className={outcome.outcome === "removed" ? "shrink-0 text-emerald-700" : outcome.outcome === "failed" ? "shrink-0 text-red-700" : "shrink-0 text-amber-700"}>{t(!outcomes ? "results.unknownItem" : outcome.outcome === "removed" && item?.action === "trash" ? "results.inTrash" : `outcome.${outcome.outcome}`)}</span></div>
        {outcome.reason ? <p className="mt-1 break-words text-xs leading-5 text-neutral-500">{translateCleanupReason(outcome.reason)}</p> : null}
        <details className="mt-2 text-xs text-neutral-500"><summary className="cursor-pointer">{t("results.originalPaths")}</summary>
          {paths.map(path => <p key={path} className="mt-2 break-all font-mono">{path}</p>)}</details>
      </li>;
    })}</ul>
    {trashAttempt ? <div className="mt-4 rounded-lg bg-sky-50 p-4 text-sm text-sky-950">
      <h4 className="font-semibold">{t("results.recoveryTitle")}</h4>
      <ol className="mt-2 list-decimal space-y-2 pl-5 text-xs leading-5"><li>{t("results.recovery1")}</li><li>{t("results.recovery2")}</li><li>{t("results.recovery3")}</li></ol>
      <p className="mt-3 text-xs leading-5">{t("results.partial")}</p><p className="mt-2 text-xs leading-5">{t("confirm.trashSpace")}</p>
    </div> : null}
  </section>;
}
