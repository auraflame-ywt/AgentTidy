// Top-level shell for the AgentTidy desktop GUI (Start.md §6, Phase 5+6).
//
// Four views (Overview / Sessions / Tidy / Diagnostics) under a single
// window. The auto-scan on mount plus the manual Refresh button both go
// through `useScanState`; the Tidy tab has its own `useCleanupState` hook
// (Phase 6) because its lifecycle is the three-step two-confirmation flow,
// not a one-shot scan. Empty installs route to the `Empty` view on the
// read-only tabs; the Tidy tab always renders so its Phase 7 rollout
// banner is reachable.
//
// i18n: this component wraps the tree in <I18nProvider> (so `main.tsx`
// stays untouched) and exposes the EN / 中文 segmented toggle in the
// header. Backend error text in the banner stays English by design.

import { useEffect, useState } from "react";
import { I18nProvider, useI18n, type Lang } from "./i18n";
import { useCleanupState, useScanState } from "./state";
import Diagnostics from "./views/Diagnostics";
import Empty from "./views/Empty";
import Overview from "./views/Overview";
import Review from "./views/Review";
import ReviewConfirm from "./views/ReviewConfirm";
import Sessions from "./views/Sessions";
import StartupScreen from "./components/StartupScreen";
import { useStartupScreen } from "./useStartupScreen";
import { DEFAULT_SESSION_QUERY, type SessionQuery } from "./session-query";
import type { AgentSnapshot, ProviderId } from "./types";

type Tab = "overview" | "sessions" | "tidy" | "diagnostics";

export default function App() {
  return (
    <I18nProvider>
      <AppShell />
    </I18nProvider>
  );
}

/** Segmented EN / 中文 control. Language names render in their own
 * language (a standard convention), so they are not translated keys. */
function LangToggle() {
  const { lang, setLang } = useI18n();
  const options: ReadonlyArray<{ id: Lang; label: string }> = [
    { id: "en", label: "EN" },
    { id: "zh", label: "中文" },
  ];
  return (
    <div
      className="flex overflow-hidden rounded-md border border-neutral-300"
      role="group"
      aria-label="Language / 语言"
    >
      {options.map((option) => (
        <button
          key={option.id}
          type="button"
          onClick={() => setLang(option.id)}
          aria-pressed={lang === option.id}
          className={`px-2 py-1 text-xs font-medium ${
            lang === option.id
              ? "bg-neutral-900 text-white"
              : "bg-white text-neutral-600 hover:bg-neutral-100"
          }`}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function AppShell() {
  const { t } = useI18n();
  const [tab, setTab] = useState<Tab>("overview");
  const [showConfirm, setShowConfirm] = useState(false);
  const { reports, snapshots, loading, error, refresh } = useScanState();
  const cleanup = useCleanupState(tab === "tidy");
  const startup = useStartupScreen(loading, error);
  const [brandPreview, setBrandPreview] = useState(false);
  const [sessionQuery, setSessionQuery] = useState<SessionQuery>(DEFAULT_SESSION_QUERY);
  const [executionSnapshots, setExecutionSnapshots] = useState<AgentSnapshot[]>([]);
  const [attemptedIds, setAttemptedIds] = useState<ReadonlySet<string>>(new Set());
  const browse = (provider?: ProviderId) => {
    setSessionQuery({ ...DEFAULT_SESSION_QUERY, provider: provider ?? "all", order: "largest" });
    setTab("sessions");
  };

  // A completed cleanup changes the disk snapshot shown in the other tabs.
  // Keep its result report visible while refreshing only the read-only data.
  useEffect(() => { if (cleanup.outcomes) refresh(); }, [cleanup.outcomes, refresh]);
  useEffect(() => {
    if (!cleanup.revalidated) setShowConfirm(false);
  }, [cleanup.revalidated, cleanup.plan?.fingerprint]);

  const tabs: ReadonlyArray<{ id: Tab; label: string }> = [
    { id: "overview", label: t("tab.overview") },
    { id: "sessions", label: t("tab.sessions") },
    { id: "tidy", label: t("tab.tidy") },
    { id: "diagnostics", label: t("tab.diagnostics") },
  ];

  const noInstallations = reports.length === 0 && snapshots.length === 0;
  const showEmpty = !loading && !error && noInstallations && tab !== "tidy";

  // Surface both scan and cleanup failures in the single banner — one
  // place to look, matching the error-handling discipline of the CLI.
  // The banner label is localized; the error body is backend English.
  const bannerError = error ?? cleanup.error;

  const selectedBytes =
    cleanup.plan
      ?.items.filter((item) => cleanup.selectedIds.has(item.unit_id))
      .reduce((sum, item) => sum + item.estimated_reclaimable_bytes, 0) ?? 0;

  if (!startup.dismissed || brandPreview) return <StartupScreen
    loading={loading} error={error} leaving={startup.leaving} preview={brandPreview}
    sessions={snapshots.reduce((sum, snapshot) => sum + snapshot.sessions.length, 0)} agents={reports.length}
    onRetry={refresh} onContinue={() => { startup.dismiss(); setBrandPreview(false); if (error) setTab("diagnostics"); }} />;

  return (
    <div className="min-h-screen bg-neutral-50 text-neutral-900">
      <header className="border-b border-neutral-200 bg-white">
        <div className="mx-auto flex max-w-6xl items-baseline justify-between px-6 py-4">
          <div className="flex items-center gap-3">
            <button type="button" onClick={() => setBrandPreview(true)} disabled={cleanup.phase === "execute"}
              title={t("startup.about")} aria-label={t("startup.about")} className="rounded-xl">
              <img src="/brand/agenttidy-icon.png" width="44" height="44" alt="" />
            </button>
            <div>
            <h1 className="text-xl font-semibold tracking-tight">
              AgentTidy
            </h1>
            <p className="text-xs text-neutral-500">{t("header.tagline")}</p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <LangToggle />
            <button
              type="button"
              onClick={() => { setShowConfirm(false); refresh(); if (cleanup.plan || cleanup.executionAttempted || tab === "tidy") cleanup.refreshPreview(); }}
              disabled={loading || cleanup.loading}
              className="rounded-md border border-neutral-300 bg-white px-3 py-1.5 text-sm font-medium text-neutral-700 hover:bg-neutral-100 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {loading ? t("common.scanning") : t("common.refresh")}
            </button>
          </div>
        </div>
        <nav className="mx-auto flex max-w-6xl gap-1 px-6 pb-3">
          {tabs.map((entry) => (
            <button
              key={entry.id}
              type="button"
              onClick={() => setTab(entry.id)}
              aria-current={tab === entry.id ? "page" : undefined}
              className={`rounded-md px-3 py-1 text-sm font-medium ${
                tab === entry.id
                  ? "bg-neutral-900 text-white"
                  : "text-neutral-600 hover:bg-neutral-100"
              }`}
            >
              {entry.label}
            </button>
          ))}
        </nav>
      </header>

      <main className="mx-auto max-w-6xl px-6 py-6">
        {bannerError ? (
          <div
            role="alert"
            className="mb-4 rounded-md border border-red-300 bg-red-50 px-4 py-3 text-sm text-red-900"
          >
            <strong className="font-semibold">{t("common.failure")}</strong>{" "}
            <span className="font-mono text-xs">{bannerError}</span>
          </div>
        ) : null}

        {loading && snapshots.length === 0 && tab !== "tidy" ? (
          <p role="status" className="rounded-xl border border-neutral-200 bg-white p-8 text-neutral-500">{t("common.scanning")}</p>
        ) : tab === "tidy" ? (
          <Review
            plan={cleanup.plan}
            snapshots={cleanup.executionAttempted ? executionSnapshots : snapshots}
            attemptedIds={attemptedIds}
            onDiagnostics={() => setTab("diagnostics")}
            loading={cleanup.loading}
            phase={cleanup.phase}
            executionAttempted={cleanup.executionAttempted}
            selectedIds={cleanup.selectedIds}
            revalidation={cleanup.revalidation}
            outcomes={cleanup.outcomes}
            revalidated={cleanup.revalidated}
            onToggle={cleanup.toggle}
            onRevalidate={cleanup.revalidate}
            onOpenConfirm={() => setShowConfirm(true)}
            onRefresh={cleanup.refreshPreview}
          />
        ) : showEmpty ? (
          <Empty />
        ) : tab === "overview" ? (
          <Overview
            reports={reports}
            snapshots={snapshots}
            plan={cleanup.executionAttempted ? null : cleanup.plan}
            loading={loading}
            onBrowse={browse}
            onReview={() => setTab("tidy")}
            onJumpToDiagnostics={() => setTab("diagnostics")}
          />
        ) : tab === "sessions" ? (
          <Sessions snapshots={snapshots} plan={cleanup.executionAttempted ? null : cleanup.plan}
            query={sessionQuery} onQueryChange={setSessionQuery} onReview={() => setTab("tidy")} />
        ) : (
          <Diagnostics reports={reports} />
        )}
      </main>

      {showConfirm && cleanup.plan ? (
        <ReviewConfirm
          key={cleanup.plan.fingerprint}
          plan={cleanup.plan}
          selectedTrashOnly={cleanup.plan.items.filter(item => cleanup.selectedIds.has(item.unit_id))
            .every(item => item.action === "trash")}
          selectedCount={cleanup.selectedIds.size}
          selectedBytes={selectedBytes}
          onConfirm={() => {
            // Keep the reviewed identities available after the successful
            // read-only refresh removes their resources from the current scan.
            setExecutionSnapshots(snapshots);
            setAttemptedIds(new Set(cleanup.selectedIds));
            setShowConfirm(false);
            cleanup.execute();
          }}
          onCancel={() => setShowConfirm(false)}
        />
      ) : null}
    </div>
  );
}