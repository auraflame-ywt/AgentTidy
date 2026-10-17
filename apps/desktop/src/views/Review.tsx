// Review & Tidy view — Start.md §6.3, Phase 6.
//
// Visual anchor: the Mac cleaner pattern in `memory/gui-mac-cleaner-
// design-language.md` — hero stat ("Found / Selected"), top-right CTA
// pair (Back / Clean Now), per-provider grouped rows of checkbox · name ·
// reason · size · risk badge. The zh locale lands on the reference
// screenshot's risk vocabulary (建议清理 / 谨慎清理 / 禁止清理) via the
// catalog while the English source copy stays canonical — satisfying the
// memory's "consistent chip text" rule for both locales.
//
// Selection rules per §6.3: low-risk default-checked, review-required
// default-unchecked, blocked never selectable. Phase 6 ships zero units
// (framework-only), so the empty state explains the Phase 7 rollout.
//
// Backend-originated text on this view: `item.reasons[0]`,
// `outcome.reason` and the error banner stay English by design (Phase 6
// renders zero units, so reasons never appear today; Phase 7 maps them
// together with the per-resource safety docs). The `outcome` enum and
// provider group headers ARE localized.

import { formatDateTime, humanBytes } from "../format";
import { useI18n } from "../i18n";
import type {
  CleanupItem,
  CleanupItemOutcome,
  CleanupOutcome,
  CleanupPlan,
  CleanupRevalidationOutcome,
  ProviderId,
  RiskLevel,
} from "../types";

type Translate = ReturnType<typeof useI18n>["t"];

/** Risk badge: green Recommended / amber Caution / gray Off-limits. */
function riskBadge(
  risk: RiskLevel,
  t: Translate,
): { label: string; className: string } {
  switch (risk) {
    case "low-risk":
      return {
        label: t("risk.lowRisk"),
        className: "bg-emerald-100 text-emerald-800",
      };
    case "review-required":
      return {
        label: t("risk.reviewRequired"),
        className: "bg-amber-100 text-amber-800",
      };
    case "blocked":
      return {
        label: t("risk.blocked"),
        className: "bg-neutral-200 text-neutral-600",
      };
  }
}

function providerLabel(provider: ProviderId, t: Translate): string {
  switch (provider) {
    case "claude-code":
      return t("provider.claudeCode");
    case "codex":
      return t("provider.codex");
    case "workbuddy":
      return t("provider.workbuddy");
  }
}

function outcomeLabel(outcome: CleanupOutcome, t: Translate): string {
  switch (outcome) {
    case "pending":
      return t("outcome.pending");
    case "removed":
      return t("outcome.removed");
    case "skipped":
      return t("outcome.skipped");
    case "failed":
      return t("outcome.failed");
  }
}

interface ReviewProps {
  plan: CleanupPlan | null;
  loading: boolean;
  selectedIds: ReadonlySet<string>;
  revalidation: CleanupRevalidationOutcome[];
  outcomes: CleanupItemOutcome[] | null;
  revalidated: boolean;
  onToggle: (unitId: string) => void;
  onRevalidate: () => void;
  /** Opens the second-confirmation modal (ReviewConfirm). */
  onOpenConfirm: () => void;
  onRefresh: () => void;
}

export default function Review({
  plan,
  loading,
  selectedIds,
  revalidation,
  outcomes,
  revalidated,
  onToggle,
  onRevalidate,
  onOpenConfirm,
  onRefresh,
}: ReviewProps) {
  const { t, lang } = useI18n();

  if (loading && !plan) {
    return <p className="text-sm text-neutral-500">{t("review.loading")}</p>;
  }

  const items = plan?.items ?? [];
  // Byte totals use the unit estimate (whole FileSet), not the primary
  // member's fingerprint size — matches plan.total_reclaimable_bytes.
  const selectedBytes = items
    .filter((item) => selectedIds.has(item.unit_id))
    .reduce((sum, item) => sum + item.estimated_reclaimable_bytes, 0);
  const staleIds = new Set(
    revalidation
      .filter((entry) => entry.outcome !== "pending")
      .map((entry) => entry.unit_id),
  );

  // Group rows by provider — the reference screenshot groups by category;
  // AgentTidy's grouping axis is the agent provider.
  const groups = new Map<string, CleanupItem[]>();
  for (const item of items) {
    const key = item.provider;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key)!.push(item);
  }

  // §12.2 plan TTL: checked at render *and* at click time — a plan that
  // expires while the view is open must never silently execute.
  const isExpired = (): boolean => {
    const expiresAt = plan?.expires_at;
    return expiresAt != null && Date.now() > expiresAt;
  };
  const expired = isExpired();

  const canExecute =
    !expired &&
    selectedIds.size > 0 &&
    revalidated &&
    [...selectedIds].every((id) => !staleIds.has(id));

  return (
    <div className="space-y-4">
      {/* Hero stat + CTA pair — the cleaner-app anchor. */}
      <div className="rounded-lg border border-neutral-200 bg-white p-5">
        <div className="flex flex-wrap items-baseline justify-between gap-4">
          <div>
            <h2 className="text-2xl font-semibold tracking-tight text-neutral-900">
              {t(
                items.length === 1 ? "review.found.one" : "review.found.other",
                {
                  bytes: humanBytes(plan?.total_reclaimable_bytes ?? 0),
                  count: items.length,
                },
              )}
            </h2>
            <p className="mt-1 text-sm text-neutral-600">
              {t("review.selected")}{" "}
              <span className="font-semibold text-emerald-700">
                {humanBytes(selectedBytes)}
              </span>{" "}
              · {t("review.planId", { id: plan?.id.slice(0, 12) ?? "" })}
              {plan?.expires_at ? (
                <span className="ml-2 text-xs text-neutral-400">
                  {t("review.expires", {
                    when: formatDateTime(plan.expires_at, lang),
                  })}
                </span>
              ) : null}
            </p>
          </div>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={onRefresh}
              className="rounded-md border border-neutral-300 bg-white px-4 py-2 text-sm font-medium text-neutral-600 hover:bg-neutral-100"
            >
              {t("review.back")}
            </button>
            {revalidated ? (
              <button
                type="button"
                onClick={() => {
                  // Live TTL re-check: expiry may pass between render and
                  // click (§12.2 — never silently execute a stale plan).
                  if (!isExpired()) onOpenConfirm();
                }}
                disabled={!canExecute}
                className="rounded-md bg-emerald-600 px-5 py-2 text-sm font-semibold text-white hover:bg-emerald-700 disabled:cursor-not-allowed disabled:opacity-40"
              >
                {t("review.cleanNow")}
              </button>
            ) : (
              <button
                type="button"
                onClick={() => {
                  if (!isExpired()) onRevalidate();
                }}
                disabled={selectedIds.size === 0 || expired}
                className="rounded-md bg-emerald-600 px-5 py-2 text-sm font-semibold text-white hover:bg-emerald-700 disabled:cursor-not-allowed disabled:opacity-40"
              >
                {t("review.reviewSelected")}
              </button>
            )}
          </div>
        </div>

        {/* Phase 6 framework-only banner (inline code stays untranslated). */}
        {items.length === 0 && !loading ? (
          <div className="mt-4 rounded-md border border-sky-200 bg-sky-50 px-4 py-3 text-sm text-sky-900">
            {t("review.phase7.before")}
            <code className="font-mono text-xs">docs/safety/</code>
            {t("review.phase7.after")}
          </div>
        ) : null}

        {/* §12.2 TTL gate — "plan expired, rescan" instead of a silent run. */}
        {expired ? (
          <div className="mt-4 rounded-md border border-amber-300 bg-amber-50 px-4 py-3 text-sm text-amber-900">
            {t("review.expired")}
          </div>
        ) : null}

        {/* §6.1 risk buckets (counts + bytes). */}
        {items.length > 0 && plan ? (
          <div className="mt-4 grid grid-cols-3 gap-3 text-center text-xs">
            <div className="rounded-md bg-emerald-50 py-2">
              <div className="font-semibold text-emerald-800">
                {t("review.bucket.recommended", {
                  count: plan.risk_summary.low_risk,
                })}
              </div>
              <div className="text-emerald-700">
                {humanBytes(plan.risk_summary.low_risk_bytes)}
              </div>
            </div>
            <div className="rounded-md bg-amber-50 py-2">
              <div className="font-semibold text-amber-800">
                {t("review.bucket.caution", {
                  count: plan.risk_summary.review_required,
                })}
              </div>
              <div className="text-amber-700">
                {humanBytes(plan.risk_summary.review_required_bytes)}
              </div>
            </div>
            <div className="rounded-md bg-neutral-100 py-2">
              <div className="font-semibold text-neutral-700">
                {t("review.bucket.offLimits", {
                  count: plan.risk_summary.blocked,
                })}
              </div>
              <div className="text-neutral-500">
                {humanBytes(plan.risk_summary.blocked_bytes)}
              </div>
            </div>
          </div>
        ) : null}
      </div>

      {/* Execution outcomes (§12.3) — per-item removed / skipped / failed. */}
      {outcomes ? (
        <div className="rounded-lg border border-neutral-200 bg-white p-4">
          <h3 className="text-sm font-semibold text-neutral-900">
            {t("review.executionResults")}
          </h3>
          <ul className="mt-2 space-y-1 text-xs">
            {outcomes.map((outcome) => (
              <li
                key={outcome.unit_id}
                className="flex items-baseline justify-between gap-2"
              >
                <span className="font-mono">{outcome.unit_id}</span>
                <span>
                  <span
                    className={
                      outcome.outcome === "removed"
                        ? "font-semibold text-emerald-700"
                        : outcome.outcome === "skipped"
                          ? "text-amber-700"
                          : "font-semibold text-red-700"
                    }
                  >
                    {outcomeLabel(outcome.outcome, t)}
                  </span>
                  {outcome.reason ? (
                    <span className="ml-2 text-neutral-500">
                      {outcome.reason}
                    </span>
                  ) : null}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {/* Per-provider grouped rows. */}
      {[...groups.entries()].map(([provider, providerItems]) => (
        <section
          key={provider}
          className="overflow-hidden rounded-lg border border-neutral-200 bg-white"
        >
          <header className="border-b border-neutral-200 bg-neutral-50 px-4 py-2 text-xs font-semibold uppercase tracking-wide text-neutral-500">
            {providerLabel(provider as ProviderId, t)}
          </header>
          <ul>
            {providerItems.map((item) => {
              const badge = riskBadge(item.risk, t);
              const blocked = item.risk === "blocked";
              const stale = staleIds.has(item.unit_id);
              const primaryReason = item.reasons[0] ?? "";
              return (
                <li
                  key={item.unit_id}
                  className="flex items-center gap-3 border-b border-neutral-100 px-4 py-3 last:border-b-0"
                >
                  <input
                    type="checkbox"
                    checked={selectedIds.has(item.unit_id)}
                    disabled={blocked || stale}
                    onChange={() => onToggle(item.unit_id)}
                    className="h-4 w-4 rounded border-neutral-300 disabled:cursor-not-allowed disabled:opacity-40"
                    aria-label={t("review.ariaSelect", { id: item.unit_id })}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="truncate font-mono text-xs text-neutral-900">
                      {item.fingerprint.path}
                    </div>
                    <div className="truncate text-xs text-neutral-500">
                      {primaryReason}
                    </div>
                    {/* §6.3 per-item facts: operation mode + recoverability,
                        plus the contained resources for FileSet units. */}
                    <div className="mt-1 flex flex-wrap items-center gap-2 text-[11px] text-neutral-500">
                      <span className="rounded bg-neutral-100 px-1.5 py-0.5">
                        {t(
                          item.action === "trash"
                            ? "review.item.action.trash"
                            : "review.item.action.providerOperation",
                        )}
                      </span>
                      <span className="rounded bg-neutral-100 px-1.5 py-0.5">
                        {t(
                          item.action === "trash"
                            ? "review.item.recoverable.trash"
                            : "review.item.recoverable.providerOperation",
                        )}
                      </span>
                      {item.locator.kind === "file-set" ? (
                        <details>
                          <summary className="cursor-pointer select-none">
                            {t(
                              item.locator.paths.length === 1
                                ? "review.item.members.one"
                                : "review.item.members.other",
                              { count: item.locator.paths.length },
                            )}
                          </summary>
                          <ul className="mt-1 space-y-0.5">
                            {item.locator.paths.map((member) => (
                              <li
                                key={member}
                                className="truncate font-mono text-[11px] text-neutral-400"
                              >
                                {member}
                              </li>
                            ))}
                          </ul>
                        </details>
                      ) : null}
                      {/* Codex ProviderOperation units carry a `db-row`
                          locator: surface the record id + database so the
                          user sees exactly which thread row is targeted
                          (docs/safety/provider-operation.md). */}
                      {item.locator.kind === "db-row" ? (
                        <span
                          className="truncate rounded bg-neutral-100 px-1.5 py-0.5 font-mono"
                          title={item.locator.database}
                        >
                          {t("review.item.record")} {item.locator.record_id}
                        </span>
                      ) : null}
                    </div>
                  </div>
                  <span className="text-xs tabular-nums text-neutral-700">
                    {humanBytes(item.estimated_reclaimable_bytes)}
                  </span>
                  <span
                    className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${badge.className}`}
                  >
                    {badge.label}
                  </span>
                  {stale ? (
                    <span className="rounded-full bg-red-100 px-2 py-0.5 text-[11px] font-medium text-red-800">
                      {t("review.stale")}
                    </span>
                  ) : null}
                </li>
              );
            })}
          </ul>
        </section>
      ))}

      {/* Step-2 hint: revalidation results summary. */}
      {revalidated && items.length > 0 ? (
        <p className="text-xs text-neutral-500">
          {staleIds.size === 0
            ? t("review.revalidatePassed")
            : t(
                staleIds.size === 1
                  ? "review.revalidateChanged.one"
                  : "review.revalidateChanged.other",
                { count: staleIds.size },
              )}
        </p>
      ) : null}
    </div>
  );
}