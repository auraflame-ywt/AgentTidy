// Static provider labels mirror the shared Rust registry (Start.md §10, §22).
import type { ProviderId } from "./types";
import type { TranslationKey } from "./i18n/en";

export const providerKeys = {
  "claude-code": "provider.claudeCode",
  codex: "provider.codex",
  workbuddy: "provider.workbuddy",
  antigravity: "provider.antigravity",
  pi: "provider.pi",
} as const satisfies Record<ProviderId, TranslationKey>;
