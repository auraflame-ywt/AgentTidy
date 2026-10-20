// Second-confirmation modal for the cleanup execute step.
//
// `docs/safety/workspace-cleanup.md` requires two independent user
// confirmations: the first is the Review view's "Review Selected" →
// revalidation; this modal is the second, shown *after* revalidation so
// the user re-confirms against fresh facts (§12.1). Displays the plan
// summary, fingerprint prefix, selected bytes, and requires an explicit
// click — no keyboard shortcut, no default focus on the confirm button.
//
// i18n: all copy is catalog-driven. The `CONFIRM` token is functional
// (it gates the execute button) and therefore intentionally NOT
// localized — only the surrounding instruction is.

import { useEffect, useRef, useState } from "react";
import { planExpired } from "../cleanup-flow";
import { usePlanExpired } from "../usePlanExpired";
import { humanBytes } from "../format";
import { useI18n } from "../i18n";
import type { CleanupPlan } from "../types";

interface ReviewConfirmProps {
  plan: CleanupPlan;
  selectedTrashOnly: boolean;
  selectedCount: number;
  selectedBytes: number;
  onConfirm: () => void;
  onCancel: () => void;
}

export default function ReviewConfirm({
  plan,
  selectedTrashOnly,
  selectedCount,
  selectedBytes,
  onConfirm,
  onCancel,
}: ReviewConfirmProps) {
  const { t } = useI18n();
  // Typed acknowledgement: the button stays disabled until the user
  // types CONFIRM — the "independent" in "two independent confirmations".
  const [typed, setTyped] = useState("");
  const expired = usePlanExpired(plan);
  const panel = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    input.current?.focus();
    return () => previous?.focus();
  }, []);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      role="dialog"
      aria-modal="true"
      aria-labelledby="cleanup-confirm-title"
      ref={panel}
      onKeyDown={event => {
        if (event.key === "Escape") { event.preventDefault(); onCancel(); }
        if (event.key !== "Tab") return;
        const controls = panel.current?.querySelectorAll<HTMLElement>("input:not(:disabled), button:not(:disabled)");
        if (!controls?.length) return;
        const first = controls[0], last = controls[controls.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
      }}
    >
      <div className="w-full max-w-md rounded-lg border border-neutral-200 bg-white p-6 shadow-xl">
        <h3
          id="cleanup-confirm-title"
          className="text-lg font-semibold text-neutral-900"
        >
          {t("confirm.title")}
        </h3>
        <p className="mt-2 text-sm text-neutral-600">
          {t(
            selectedCount === 1 ? "confirm.body.one" : "confirm.body.other",
            {
              count: selectedCount,
              bytes: humanBytes(selectedBytes),
            },
          )}
        </p>
        <p className="mt-2 text-xs text-amber-800">{t(selectedTrashOnly ? "review.item.recoverable.trash" : "confirm.providerBody")}</p>
        {selectedTrashOnly ? <p className="mt-2 text-xs text-neutral-500">{t("confirm.trashSpace")}</p> : null}
        <dl className="mt-3 space-y-1 rounded-md bg-neutral-50 p-3 text-xs text-neutral-700">
          <div className="flex justify-between gap-2">
            <dt className="text-neutral-500">{t("confirm.plan")}</dt>
            <dd className="font-mono">{plan.id.slice(0, 16)}</dd>
          </div>
          <div className="flex justify-between gap-2">
            <dt className="text-neutral-500">{t("confirm.fingerprint")}</dt>
            <dd className="font-mono">{plan.fingerprint.slice(0, 12)}…</dd>
          </div>
          <div className="flex justify-between gap-2">
            <dt className="text-neutral-500">{t("confirm.reclaimable")}</dt>
            <dd>{humanBytes(selectedBytes)}</dd>
          </div>
        </dl>
        <label className="mt-4 block text-xs text-neutral-600">
          {t("confirm.typeBefore")}
          <span className="font-mono font-semibold">CONFIRM</span>
          {t("confirm.typeAfter")}
          <input
            type="text"
            ref={input}
            value={typed}
            onChange={(event) => setTyped(event.target.value)}
            className="mt-1 w-full rounded-md border border-neutral-300 px-3 py-2 text-sm focus:border-emerald-500 focus:outline-none"
            autoComplete="off"
            spellCheck={false}
          />
        </label>
        {expired ? <p role="alert" className="mt-3 text-sm text-amber-800">{t("review.expired")}</p> : null}
        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-md border border-neutral-300 bg-white px-4 py-2 text-sm font-medium text-neutral-700 hover:bg-neutral-100"
          >
            {t("confirm.cancel")}
          </button>
          <button
            type="button"
            onClick={() => { if (typed === "CONFIRM" && selectedCount > 0 && !planExpired(plan)) onConfirm(); }}
            disabled={typed !== "CONFIRM" || expired || selectedCount === 0}
            className="rounded-md bg-emerald-600 px-5 py-2 text-sm font-semibold text-white hover:bg-emerald-700 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {t(selectedTrashOnly ? "confirm.moveToTrash" : "confirm.execute")}
          </button>
        </div>
      </div>
    </div>
  );
}