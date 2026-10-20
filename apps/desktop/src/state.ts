// Stable page-state imports (Start.md §6). Independent read-only and mutation
// lifecycles live in focused modules so refreshes cannot reset confirmation.
export { useScanState, type ScanState } from "./useScanState";
export { useCleanupState, type CleanupState } from "./useCleanupState";
