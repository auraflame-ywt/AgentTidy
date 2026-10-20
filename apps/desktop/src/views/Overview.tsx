// Decision-oriented storage overview (Start.md §6.1). Measured resource
// footprint and plan estimates stay separate; neither promises freed space.
import { humanBytes, formatDateTime } from "../format";
import { useI18n } from "../i18n";
import { diagnosticCount, storageSummary } from "../presentation";
import { usePlanExpired } from "../usePlanExpired";
import type { AgentSnapshot, CleanupPlan, DoctorReport, ProviderId } from "../types";

const providerKeys = { "claude-code": "provider.claudeCode", codex: "provider.codex", workbuddy: "provider.workbuddy" } as const;
interface OverviewProps {
  reports: DoctorReport[]; snapshots: AgentSnapshot[]; plan: CleanupPlan | null;
  loading: boolean; onBrowse: (provider?: ProviderId) => void;
  onReview: () => void; onJumpToDiagnostics: () => void;
}

export default function Overview({ reports, snapshots, plan, loading, onBrowse, onReview, onJumpToDiagnostics }: OverviewProps) {
  const { t, lang } = useI18n();
  const expired = usePlanExpired(plan);
  const summary = storageSummary(snapshots);
  const diagnostics = reports.reduce((sum, report) => sum + diagnosticCount(report,
    snapshots.find(snapshot => snapshot.installation.id === report.installation.id)), 0);
  const ranked = reports.map(report => {
    const snapshot = snapshots.find(value => value.installation.id === report.installation.id);
    return { report, snapshot, bytes: snapshot?.resources.reduce((sum, resource) => sum + resource.size.logical_bytes, 0) ?? 0 };
  }).sort((a, b) => b.bytes - a.bytes);
  const validPlan = plan && !expired ? plan : null;
  if (loading && snapshots.length === 0) return <p role="status" className="rounded-xl border border-neutral-200 bg-white p-8 text-neutral-500">{t("common.scanning")}</p>;
  return <div className="space-y-6">
    <section className="rounded-2xl bg-neutral-900 p-6 text-white sm:p-8">
      <p className="text-sm text-neutral-300">{t("home.eyebrow")}</p>
      <div className="mt-3 flex flex-wrap items-end justify-between gap-6">
        <div><h2 className="text-4xl font-semibold tracking-tight sm:text-5xl">{humanBytes(summary.bytes)}</h2>
          <p className="mt-3 text-sm text-neutral-300">{t("home.scope", { agents: reports.length, sessions: summary.sessions })}{summary.estimated ? ` · ${t("confidence.estimated")}` : ""}</p></div>
        <button type="button" onClick={() => onBrowse()} className="rounded-lg bg-white px-4 py-2.5 text-sm font-semibold text-neutral-900">{t("home.browseLargest")} →</button>
      </div>
      <p className="mt-6 border-t border-white/15 pt-4 text-xs leading-5 text-neutral-400">{t("home.measurement")}</p>
    </section>
    <div className="grid gap-4 sm:grid-cols-3">
      <button type="button" onClick={onReview} className="rounded-xl border border-emerald-200 bg-emerald-50 p-5 text-left hover:border-emerald-400">
        <span className="text-xs font-medium text-emerald-800">{t("home.reviewable")}</span>
        <span className="mt-2 block text-2xl font-semibold text-emerald-950">{validPlan ? humanBytes(validPlan.total_reclaimable_bytes) : "—"}</span>
        <span className="mt-2 block text-xs leading-5 text-emerald-800">{validPlan ? t("home.reviewHint") : t("home.previewNeeded")} →</span>
      </button>
      <button type="button" onClick={onReview} className="rounded-xl border border-neutral-200 bg-white p-5 text-left hover:border-neutral-400">
        <span className="text-xs font-medium text-neutral-500">{t("home.protected")}</span>
        <span className="mt-2 block text-2xl font-semibold">{validPlan ? validPlan.risk_summary.blocked : "—"}</span>
        <span className="mt-2 block text-xs leading-5 text-neutral-500">{t("home.protectedHint")} →</span>
      </button>
      <button type="button" onClick={onJumpToDiagnostics} className="rounded-xl border border-neutral-200 bg-white p-5 text-left hover:border-neutral-400">
        <span className="text-xs font-medium text-neutral-500">{t("home.diagnostics")}</span>
        <span className="mt-2 block text-2xl font-semibold">{diagnostics}</span>
        <span className="mt-2 block text-xs leading-5 text-neutral-500">{t("home.diagnosticsHint")} →</span>
      </button>
    </div>
    <section>
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2"><h3 className="font-semibold">{t("home.byAgent")}</h3>
        <span role="status" className="text-xs text-neutral-500">{loading ? t("common.scanning") : summary.scannedAt == null ? "—" : t("home.scanned", { when: formatDateTime(summary.scannedAt, lang) })}</span></div>
      <div className="divide-y divide-neutral-100 overflow-hidden rounded-xl border border-neutral-200 bg-white">
        {ranked.map(({ report, snapshot, bytes }) => {
          const count = diagnosticCount(report, snapshot);
          return <article key={report.installation.id} className="p-5">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div><h4 className="font-semibold">{t(providerKeys[report.installation.provider])}</h4>
                <p className="mt-1 text-xs text-neutral-500">{snapshot ? t("home.sessionCount", { count: snapshot.sessions.length }) : t("detail.unknown")} · {t(`status.${report.installation.status === "permission-required" ? "permissionRequired" : report.installation.status}`)}</p></div>
              <div className="flex items-center gap-5"><span className="text-lg font-semibold tabular-nums">{snapshot ? humanBytes(bytes) : "—"}</span>
                <button type="button" onClick={() => onBrowse(report.installation.provider)} className="rounded-md border border-neutral-200 px-3 py-2 text-xs font-medium">{t("home.viewSessions")}</button></div>
            </div>
            <div className="mt-4 h-1.5 overflow-hidden rounded-full bg-neutral-100" aria-hidden="true"><div className="h-full rounded-full bg-emerald-500" style={{ width: `${summary.bytes ? bytes / summary.bytes * 100 : 0}%` }} /></div>
            <div className="mt-3 flex flex-wrap items-center gap-3 text-xs text-neutral-500">
              <span>{t(report.inspection.agent_running ? "home.running" : "home.idle")}</span>
              {count > 0 ? <button type="button" onClick={onJumpToDiagnostics} className="text-amber-800 underline underline-offset-2">{t("home.attention", { count })}</button> : null}
              <details className="ml-auto"><summary className="cursor-pointer">{t("detail.technical")}</summary><p className="mt-2 break-all font-mono">{report.installation.id}</p>
                {report.installation.data_roots.map(root => <p key={root} className="mt-1 break-all font-mono">{root}</p>)}</details>
            </div>
          </article>;
        })}
      </div>
    </section>
    <p className="text-xs text-neutral-500">{t("home.privacy")}</p>
  </div>;
}
