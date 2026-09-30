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
| Provider DB records (`provider-operation`) | Documented in [`provider-operation.md`](./provider-operation.md); **enabled (Phase 7 beta)** for the Codex provider's archived sessions only. |