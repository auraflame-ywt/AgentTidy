// Common verified backend explanations mapped to user-facing copy (§6.3).
// Unknown provider/OS failures retain their details rather than a guessed label.
import type { TranslationKey } from "./i18n/en";
export function cleanupReasonKey(reason: string): TranslationKey | null {
  // Admission failures need their own copy: refreshing or closing an agent
  // cannot establish complete recovery or install a supported native CLI.
  if (reason.includes("Codex cleanup disabled: complete session recovery"))
    return reason.includes("Codex CLI unavailable:") || reason.includes("Codex CLI unsupported:")
      ? "cleanup.reason.codexRecoveryCli" : "cleanup.reason.codexRecovery";
  if (reason.startsWith("agent is currently running") || reason.startsWith("agent is running"))
    return "cleanup.reason.running";
  if (reason.startsWith("scan reported an Error problem")) return "cleanup.reason.scanError";
  if (reason.startsWith("provider safety validation failed") || reason.startsWith("cannot prove complete member evidence"))
    return "cleanup.reason.unverified";
  if (reason.startsWith("session ") && reason.includes(" is Inactive")) return "cleanup.reason.inactive";
  if (reason.startsWith("session ") && reason.includes("Archived but recently active")) return "cleanup.reason.recentArchive";
  if (reason.startsWith("session ") && reason.includes("Archived with unknown last-activity")) return "cleanup.reason.unknownArchive";
  if (reason.startsWith("all §7.1 conditions hold")) return "cleanup.reason.archivedOld";
  if (reason.startsWith("plan expired")) return "review.expired";
  if (reason.startsWith("moved to system Trash;")) return "cleanup.reason.inTrash";
  return null;
}
