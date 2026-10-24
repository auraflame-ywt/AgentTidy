# AgentTidy

> Keep your AI agents tidy.

**Local-first AI Agent Session & Storage Manager** — an open-source tool that
discovers, understands, browses and — safely — tidies up the local data your
AI agents leave behind on your machine.

AgentTidy is not a disk cleaner. It first answers:

1. Which agents stored data on this machine?
2. Where, and how much space does it take?
3. Which sessions, projects and resources does it belong to?
4. What can be handled safely, what needs review, and what is off-limits?

Core principles: **Local-first · Read-first · Trash-first · Fail-closed.**
See [`Start.md`](./Start.md) for the full v0.1 design document (Chinese).

## Status

Pre-release, **Phase 7 cleanup Beta under validation**. Read-only detection,
scanning, sessions and diagnostics are implemented for **WorkBuddy**,
**Claude Code**, **Codex**, **Antigravity** and **Pi**, through a shared Rust API,
CLI and Tauri GUI. Antigravity / Pi currently support macOS read-only analysis;
see [provider scope](docs/providers/antigravity.md) and [Pi scope](docs/providers/pi.md).
Claude Code / WorkBuddy session file sets support guarded Trash cleanup;
Codex archived-session candidates remain Blocked until native deletion and
complete session recovery pass acceptance; CLI admission is shown in preview
and Diagnostics.
Default workspaces remain report-only. Real-user Beta and macOS / Windows
recovery and compatibility acceptance are still pending.

Local Mac scenario coverage and remaining gates are recorded in
[macOS acceptance](./docs/testing/macos-scenarios.md).

The Mac desktop now includes storage ranking, searchable sessions with resource
details, cleanup receipts and a unified app icon/startup screen. The latest
checks passed 165 Rust tests and 17 frontend regressions; these local results
do not replace real-user cleanup/recovery acceptance. See the
[2026-10-20 progress summary](./docs/CHANGELOG.md)
and [brand assets](./apps/desktop/assets/BRAND.md).

See [project progress](./docs/CHANGELOG.md) and the
[development phases](./Start.md#19-开发顺序).

## Architecture

```text
┌─────────────────────────────┐
│ Presentation                │
│ Desktop GUI / CLI           │
└──────────────┬──────────────┘
               │
┌──────────────▼──────────────┐
│ Application API             │
│ Detect / Scan / List        │
│ Plan / Validate / Execute   │
└──────────────┬──────────────┘
               │
┌──────────────▼──────────────┐
│ Core                        │
│ Models / Policy / Planning  │
└──────────────┬──────────────┘
     ┌─────────┴─────────┐
┌────▼─────────┐  ┌──────▼────────┐
│ Providers    │  │ Infrastructure│
│ WB/Claude/   │  │ FS/SQLite/     │
│ Codex        │  │ Trash/Process  │
└──────────────┘  └───────────────┘
```

## Repository layout

```text
apps/desktop/    Tauri 2 + React desktop GUI
apps/cli/        agenttidy CLI
crates/          Rust core, application API, infrastructure, provider contracts
providers/       Per-agent adapters (workbuddy, claude-code, codex, antigravity, pi)
fixtures/        Sanitized test fixtures
docs/            Provider investigation docs, safety notes, ADRs
```

## Development

Requirements: Rust (stable), Node.js ≥ 20, pnpm.

```bash
pnpm install                     # frontend dependencies
cargo build --workspace          # build all Rust crates
cargo test --workspace           # run tests
cargo run -p agenttidy-cli       # run the CLI
pnpm test:desktop                # session / confirmation regression tests
pnpm tauri dev                   # run the desktop app
```

## License

[MIT](./LICENSE) — see [CHANGELOG.md](./CHANGELOG.md) for release history.
