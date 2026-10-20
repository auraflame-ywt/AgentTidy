// Brand entrance and real startup status (Start.md §6.1 / §13). No artificial
// scan percentages or minimum splash delay; metadata loading controls exit.
import { useI18n } from "../i18n";

export default function StartupScreen({ loading, error, leaving, preview, sessions, agents, onContinue, onRetry }: {
  loading: boolean; error: string | null; leaving: boolean; preview: boolean;
  sessions: number; agents: number; onContinue: () => void; onRetry: () => void;
}) {
  const { t } = useI18n();
  return <main className={`startup-screen${leaving && !preview ? " startup-screen--leaving" : ""}`} aria-label={t("startup.label")}>
    <div className="startup-grid" aria-hidden="true" />
    <header className="startup-top">
      <div className="startup-wordmark"><img src="/brand/agenttidy-icon.png" width="36" height="36" alt="" /><span>AgentTidy</span></div>
      <span className="startup-edition">LOCAL EDITION <span className="startup-edition-dot">·</span> v0.1.0</span>
    </header>
    <section className="startup-hero">
      <div className="startup-copy">
        <p className="startup-eyebrow"><span /> A LITTLE ORDER. A LOT OF CLARITY.</p>
        <h1>{t("startup.headline1")}<br /><span>{t("startup.headline2")}</span></h1>
        <p className="startup-description">{t("startup.description")}</p>
        <div className="startup-principles"><span>{t("startup.local")}</span><span>{t("startup.private")}</span><span>{t("startup.intentional")}</span></div>
      </div>
      <div className="startup-art" aria-hidden="true">
        <div className="startup-art-glow" /><div className="startup-frame startup-frame--outer" /><div className="startup-frame startup-frame--inner" />
        <span className="startup-cross startup-cross--a">+</span><span className="startup-cross startup-cross--b">+</span>
        <img className="startup-icon" src="/brand/agenttidy-icon.png" width="320" height="320" alt="" />
        <span className="startup-art-caption">FROM SCATTERED TO STRUCTURED</span>
      </div>
    </section>
    <footer className="startup-bottom">
      <div className="startup-status-area">
        <div className="startup-status" role="status" aria-live="polite">
          <span className={`startup-indicator${loading ? " startup-indicator--loading" : ""}${error ? " startup-indicator--error" : ""}`} aria-hidden="true" />
          <div><p>{error ? t("startup.error") : loading ? t("startup.scanning") : t("startup.ready", { agents, sessions })}</p>
            <p className="startup-status-note">{error ? t("startup.errorHint") : t("startup.readonly")}</p></div>
        </div>
        {error ? <details className="startup-error"><summary>{t("detail.technical")}</summary><p>{error}</p></details> : null}
      </div>
      <div className="startup-actions">
        {error ? <button type="button" className="startup-retry" onClick={onRetry}>{t("startup.retry")}</button> : null}
        <button type="button" className="startup-enter" onClick={onContinue}>{t(error ? "startup.inspect" : loading ? "startup.background" : "startup.enter")}<span aria-hidden="true">↗</span></button>
      </div>
    </footer>
    <div className="startup-colophon"><span>DISCOVER / UNDERSTAND / TIDY</span><span>{t("startup.signature")}</span></div>
  </main>;
}
