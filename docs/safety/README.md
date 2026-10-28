# Safety Notes

Path security rules, active-data protection, operation logging and recovery
semantics (design doc §16). The general cleanup execution contract — risk
vocabulary, revalidation triggers, two confirmations, audit log — lives in
[`workspace-cleanup.md`](./workspace-cleanup.md) and applies to every
cleanup-capable resource type.

Each cleanup-capable resource type must get its own document here before
its cleanup is enabled. Current status:

| Resource type | Status |
| --- | --- |
| Default workspaces (`file-tree`) | Documented; **not enabled** (Phase 6 framework only, enablement deferred — see the doc's status note). |
| Session transcripts (`file-set`) | Documented in [`session-transcript-fileset.md`](./session-transcript-fileset.md); **enabled (Phase 7 beta)** for the Claude Code provider. Documented in [`workbuddy-session-fileset.md`](./workbuddy-session-fileset.md); **enabled (Phase 7 beta)** for the WorkBuddy provider. |
| Provider DB records (`provider-operation`) | Documented in [`provider-operation.md`](./provider-operation.md); **Blocked pending complete recovery acceptance** for Codex archived sessions (CLI admission diagnosed early; native deletion and DB replacement disabled). |
| Antigravity session store | Documented in [`antigravity-session-store.md`](./antigravity-session-store.md); **not enabled** — no cleanup unit is produced and lifecycle stays `Unknown` on both platforms. |
| Pi session file set | Documented in [`pi-session-fileset.md`](./pi-session-fileset.md); **not enabled** — no cleanup unit is produced. Pi is additionally not discoverable on Windows, because that platform has no sample yet. |

## Platform acceptance

Native recovery can only be certified per platform, and a result on one
platform is never a result on another.

| Platform | Native trash round trip | Controlled cleanup loop | Notes |
| --- | --- | --- | --- |
| macOS | Accepted (Finder Trash via NSFileManager) | Accepted | `crates/infrastructure/src/macos_acceptance.rs` |
| Windows | Accepted (Recycle Bin `$I`/`$R` pair) | Accepted (WorkBuddy `file-set`) | `crates/infrastructure/src/windows_acceptance.rs`, `crates/application/src/tests/cleanup_acceptance.rs` |

Both native tests are opt-in (`--ignored --exact`) and touch only their
own uniquely named samples; a platform refusal fails the test rather than
downgrading it to a skip. The controlled cleanup acceptance runs the whole
plan → revalidate → execute → restore loop against a synthetic provider
installation under an isolated home, so it never touches real user data.
Scenario records: [`../testing/macos-scenarios.md`](../testing/macos-scenarios.md)
and [`../testing/windows-scenarios.md`](../testing/windows-scenarios.md).