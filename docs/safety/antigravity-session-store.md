# Antigravity session store: coordinated cleanup

Status: implementation and isolated native recovery acceptance in progress.
Scope: observed macOS Antigravity 2.24.0 / language server 1.11.0 formats.
References: Start.md §10, §12.4, §16, §18, §22.

Native isolated tests verified that DeleteCascadeTrajectory removes the target
legacy `.pb` or current `.db` (including existing WAL/SHM) and its annotation,
removes the conversation index row/search projection, and updates the shared
`agyhub_summaries_proto.pb` map. Neighboring index rows remain unchanged.
Native deletion preserves `brain/` user artifacts; AgentTidy also preserves them.

The provider operation coordinates a *complete* explicit file set: shared index,
existing shared summary map, one verified trajectory and its optional annotation.
Shared stores are not independently trashable. The operation is never labeled
as a simple Move to Trash. The existing Application policy, complete evidence,
two confirmations, operation lock and synced audit apply unchanged.

Only verified ids/formats, independent top-level sessions and single-link files
can form units. Unknown schemas/triggers, duplicate formats, fork/subagent
relationships, unreadable annotations, linked ancestors, active writers and
remaining WAL/SHM block execution. Close Antigravity normally so it checkpoints
before cleaning. Credentials, config, projects and brain files are not targets.

Before mutation, a private recovery directory retains exact pre-images of every
member plus a metadata-only manifest. A prepared index copy is changed inside a
SQLite transaction (target FTS rows and summary row), checked and compacted. The
summary map drops only the verified target key and preserves all neighboring
entry bytes. Source databases are never opened writable. Source hashes and
process absence are checked again before committing prepared files.

The source index and map are replaced separately, then session-owned files are
moved to native Trash. Cross-file updates are **not atomic**. Any failure reports
Failed, preserves the recovery folder and records whether any source changed;
no partial result is labeled successful. Physical reclaimed bytes remain zero:
Trash and retained pre-images still occupy disk. Recovery requires the complete
manifest, never only the shared SQLite snapshot.

Manual recovery: close Antigravity, verify no subsequent sessions/operations
have changed the shared stores, retain the current state, restore every
manifest pre-image to its original location (remove stale WAL/SHM only while
closed), then reopen Antigravity and verify the session, archive state, search
index and unchanged neighboring sessions. Never overwrite newer work. A native
roundtrip on an isolated profile is required before enabling this path.
