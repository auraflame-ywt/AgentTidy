// Hand-rolled i18n runtime (zh/en) — no external dependency, per the
// approved i18n plan. Design:
//
// - `en.ts` is the source of truth; `zh.ts` is `Record<TranslationKey,
//   string>`, so `tsc -b` (part of `pnpm build:desktop`) fails on any
//   missing or extra key. Catalog completeness is a build gate, not a
//   runtime concern.
// - `t(key, params)` substitutes `{name}` placeholders; a missing lookup
//   falls back to English defensively (the type layer already forbids it).
// - First launch detects `navigator.language` (zh-* → zh, else en);
//   the user's choice persists in `localStorage["agenttidy.lang"]` and is
//   mirrored to `document.documentElement.lang` (overriding the static
//   `lang="en"` in index.html).
// - `translateDiagnostic(code, fallback)` maps backend ScanProblem codes
//   (fixed or path-suffixed) to Chinese with verbatim dynamic suffixes;
//   unknown codes and the English UI keep the backend message as-is.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { en, type TranslationKey } from "./en";
import { cleanupReasonKey } from "../cleanup-reasons";
import { zh } from "./zh";

export type { TranslationKey };
export type Lang = "en" | "zh";

/** `{name}` placeholder values accepted by `t`. */
export type TranslationParams = Record<string, string | number>;

const STORAGE_KEY = "agenttidy.lang";

/** Catalogs for every supported language. Adding a language = one file +
 * one entry here; `Record<TranslationKey, string>` keeps it complete. */
const CATALOGS: Record<Lang, Partial<Record<TranslationKey, string>>> = {
  en,
  zh,
};

function detectLang(): Lang {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (stored === "en" || stored === "zh") return stored;
  } catch {
    // localStorage can be unavailable (privacy settings); fall through.
  }
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

/** `Intl` locale tag for the active language. */
function localeTag(lang: Lang): string {
  return lang === "zh" ? "zh-CN" : "en-US";
}

interface I18nContextValue {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: (key: TranslationKey, params?: TranslationParams) => string;
  /** Map a backend ScanProblem code to localized text (zh), falling back
   * to the backend's English message for en or unknown codes. */
  translateDiagnostic: (code: string, fallback: string) => string;
  translateCleanupReason: (reason: string) => string;
}

const I18nContext = createContext<I18nContextValue | null>(null);

/** Dynamic `ScanProblem` code stems whose `:{suffix}` (path / id) is a
 * stable part of the key on the Rust side but never translated. */
const DYNAMIC_CODE_STEMS = [
  "unreadable",
  "unrecognized-transcript",
  "unrecognized-rollout",
  "duplicate-session",
  "thread-index-missing-session",
  "archive-state-mismatch",
  "project-unreadable",
  "workspace-unreadable",
  "pi-header",
  "pi-duplicate",
  "pi-walk",
  "antigravity-invalid-id",
  "antigravity-directory",
  "antigravity-transcript-missing",
  "antigravity-workspaces",
  "antigravity-walk",
] as const;

function lookup(
  lang: Lang,
  key: TranslationKey,
  params?: TranslationParams,
): string {
  const catalog = CATALOGS[lang];
  // Type layer guarantees presence; English fallback is defense in depth.
  const template = catalog[key] ?? en[key];
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (placeholder, name: string) => {
    const value = params[name];
    return value === undefined ? placeholder : String(value);
  });
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(detectLang);

  // Mirror the active language onto <html lang> (index.html ships a
  // static lang="en" that must follow runtime switches).
  useEffect(() => {
    document.documentElement.lang = localeTag(lang);
  }, [lang]);

  const setLang = useCallback((next: Lang) => {
    setLangState(next);
    try {
      window.localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Persistence is best-effort; the in-memory switch still works.
    }
  }, []);

  const value = useMemo<I18nContextValue>(
    () => ({
      lang,
      setLang,
      t: (key, params) => lookup(lang, key, params),
      translateCleanupReason: reason => {
        const key = cleanupReasonKey(reason);
        return key ? lookup(lang, key) : reason;
      },
      translateDiagnostic: (code, fallback) => {
        if (lang === "en") return fallback;
        // Exact match on a stable code first.
        const exact = `problem.${code}` as TranslationKey;
        if (exact in zh) return lookup(lang, exact);
        // Then known dynamic stems: translate the stem, append the
        // path/id suffix verbatim.
        for (const stem of DYNAMIC_CODE_STEMS) {
          if (code.startsWith(`${stem}:`)) {
            const suffix = code.slice(stem.length + 1);
            const key = `problem.${stem}` as TranslationKey;
            if (key in zh) return `${lookup(lang, key)} ${suffix}`;
          }
        }
        return fallback;
      },
    }),
    [lang, setLang],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

/** Access the i18n context. Every component below the provider uses this. */
export function useI18n(): I18nContextValue {
  const value = useContext(I18nContext);
  if (!value) {
    // Programming error: a view rendered outside <I18nProvider>.
    throw new Error("useI18n must be used inside <I18nProvider>");
  }
  return value;
}