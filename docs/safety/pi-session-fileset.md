# Pi session files: Trash cleanup and recovery

Status: macOS implementation / isolated native acceptance in progress.
Applies to Pi 0.87.1 v3 sessions (`Start.md` §10, §12, §16.1–16.4, §18).

The installed `session-selector.js::deleteSessionFile` deletes one selected
JSONL file, preferring Trash. AgentTidy uses Trash exclusively, with no unlink
fallback. `SessionManager.forkFrom` copies all non-header entries into the new
file; `parentSession` is ancestry metadata, not a requirement to load the body.
No archive state exists in the investigated session format/UI. Archive and
RestoreArchive remain explicitly unsupported rather than inventing a lifecycle.

A cleanup unit contains exactly one v3 session JSONL file in the detected
session directory, with its native `_<header-id>.jsonl` suffix. It must be a
regular file with exactly one hard link. Duplicate ids, unknown formats,
non-native filenames, unreadable data and redirected roots cannot be cleaned.
The project cwd, auth/config, extensions and package data are never members.

The adapter reports Inactive only with no detected Pi process; running or
unobservable process state stays Unknown. Retention/recommendation stays in
shared policy: inactive unarchived sessions require explicit review, and age
alone cannot make them LowRisk. Preview and execute both re-scan, validate
header/id/path/ownership/lifecycle and compare complete file metadata evidence.
Validation rejects extra locator members and forged units. A new Pi writer,
file replacement or header/size/mtime change invalidates the preview.

Execution uses the existing cross-process lock, synced audit intent, two UI
confirmations and NSFileManager Trash. Manual recovery moves the exact file
back to its recorded original path while Pi is closed; never overwrite a new
file at that path. Reopen with Pi's own SessionManager to verify id, cwd and
metadata. No in-app Undo or permanent deletion is introduced.
