// Accessible read-only inspection panel (Start.md §6.2 / §6.3). Native dialog
// supplies focus containment, Escape and inert background in the Mac WebView.
import { useEffect, useRef, type ReactNode } from "react";
import { useI18n } from "../i18n";

export default function DetailPanel({ title, children, onClose }: {
  title: string; children: ReactNode; onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const { t } = useI18n();
  useEffect(() => {
    const element = dialog.current;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    element?.showModal();
    // React removes the dialog on close, so explicitly restore the opener
    // instead of depending on the browser's detached-dialog focus behavior.
    return () => { element?.close(); if (previous?.isConnected) previous.focus(); };
  }, []);
  return <dialog ref={dialog} aria-labelledby="session-detail-title"
    onCancel={event => { event.preventDefault(); onClose(); }}
    className="m-0 ml-auto h-dvh max-h-none w-full max-w-xl border-0 bg-white p-0 text-neutral-900 shadow-2xl backdrop:bg-neutral-950/30">
    <div className="sticky top-0 z-10 flex items-start justify-between gap-4 border-b border-neutral-200 bg-white px-6 py-5">
      <h2 id="session-detail-title" className="break-words text-lg font-semibold">{title}</h2>
      <button type="button" autoFocus onClick={onClose} className="shrink-0 rounded-md border border-neutral-300 px-3 py-1.5 text-sm">{t("detail.close")}</button>
    </div>
    <div className="space-y-6 p-6">{children}</div>
  </dialog>;
}
