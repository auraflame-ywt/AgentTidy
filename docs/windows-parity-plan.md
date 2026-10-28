# Windows 对齐 macOS 实施计划

制定日期：2026-10-24。本机：Windows 10/11（`C:\Users\18712`），Rust 1.9x，
Rust 工作区 + pnpm 工作区。所有结论均在本机复现并附证据；未在 macOS 上重新验证。

本计划的目标是让 Windows 端达到 macOS 端当前的**能力与安全门槛**，
而不是"能编译"。macOS 端当前状态见 [`CHANGELOG.md`](./CHANGELOG.md) 与
[`testing/macos-scenarios.md`](./testing/macos-scenarios.md)。

边界（与仓库红线一致）：默认只读、Trash-first、Fail-closed；
平台分支只允许出现在 `crates/infrastructure` 与 provider 路径发现；
未知资源一律 Blocked；每个可清理资源类型必须先有 `docs/safety/` 文档。

---

## 0. 本机实测基线（2026-10-24）

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| 格式 | `cargo fmt --all --check` | 通过 |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | **失败**（D5） |
| Rust 测试 | `cargo test --workspace` | **失败**（D3、D4） |
| 前端类型 + 构建 | `pnpm build:desktop` | 通过 |
| 前端回归 | `pnpm test:desktop` | **失败**（D6） |
| CLI 只读端到端 | `agenttidy doctor --json` / `scan --json` | 通过，检出 4 个安装 |
| 桌面运行时 | WebView2 155.0.4283.62 | 已安装 |

CLI 在本机成功检出：`claude-code:cli`、`claude-code:desktop`、
`codex:default`、`workbuddy:default`（共 45 个会话）。
Antigravity 与 Pi 在 Windows 上被硬关闭（D10）。

---

## 1. 阻塞性缺陷（必须先修，否则 Windows CI 一直是红的）

### D1 `UsageAccumulator.allocated_bytes` 恒为 `None`（跨平台）

- 位置：`crates/infrastructure/src/disk_usage.rs`
- 成因：`allocated_bytes: Option<u64>` 由 `#[derive(Default)]` 得到 `None`，
  而首次累加执行 `opt_add_for_usage(None, Some(x))` → `None`，
  此后无论每个文件是否可测量，聚合值永远是 `None`。
- 证据：本机 56 254 次 `fs_probe::allocated_bytes()` 调用**全部**返回 `Some(..)`，
  但每个 provider 资源仍输出 `"allocated_bytes": null`。
- 影响：§8 的"逻辑大小 / 分配大小 / 独占"口径从未真正生效，
  GUI 与 CLI 的占用数字在所有平台都只有逻辑大小。
- 修复：`UsageAccumulator` 改为手写 `Default`（`allocated_bytes: Some(0)`），
  保持"任一贡献不可测量则整体不可测量"的原意不变。
- 回归：为 `UsageAccumulator` 增加"全部可测量 → Some(总和)"与
  "存在不可测量 → None"两条单测。

### D2 只读 SQLite 打开是惰性的，导致选错读取档位

- 位置：`crates/infrastructure/src/sqlite.rs::open_readonly`
- 成因：`mode=ro` 打开一个 **WAL 模式且 `-shm` 不存在** 的库时，
  `open_with_flags` 会**惰性成功**；直到第一次真正执行语句才会报
  `SQLITE_CANTOPEN`（error 14）。`ReadOnlyDb::open` 因此认为直读成功，
  从不落到"私有副本"降级路径。
- 证据（本机 `workbuddy.db`，WAL 模式、WorkBuddy 未运行故 `-wal`/`-shm` 已回收）：
  直读 `PRAGMA journal_mode` → `CannotOpen(14) unable to open database file`；
  私有副本 → `quick_check = ok`、`sessions` 44 行。
- 影响：WorkBuddy 生命周期推断被跳过（`workbuddy.db failed quick_check;
  lifecycle inference skipped (fail-closed)`），
  本机 41 个 WorkBuddy 会话全部 `lifecycle: unknown`，
  **Phase 7 的 WorkBuddy FileSet Beta 在 Windows 上产出 0 个可执行单元**。
  同一缺陷在 macOS 上只要 Agent 未运行（`-shm` 已回收）同样触发。
- 修复：`open_readonly` 在返回 `Ok` 前必须执行一次真实查询
  （如 `SELECT count(*) FROM sqlite_master`）验证可读；
  失败即走既有私有副本降级。
- 回归：新增"无 `-shm` 的 WAL 库"用例，断言 `origin() == PrivateCopy`
  且 `quick_check_ok()` 为真。

### D3 `MutableSqliteDb::drop` 在 Windows 上泄漏临时目录

- 位置：`crates/infrastructure/src/sqlite.rs`
- 成因：注释声称"先关连接再删目录"，但 `self.conn` 在 `Drop::drop`
  执行期间仍然存活（字段在 `drop` 之后才析构）。Windows 上
  `remove_dir_all` 因 `ERROR_SHARING_VIOLATION` 失败。
- 影响：测试 `mutable_sqlite_db_drop_removes_temp_directory` 失败；
  生产上每次 `MutableSqliteDb` 落盘都会留下一个临时目录。
- 修复：与同文件 `ReadOnlyDb::drop` 保持一致——先用
  `Connection::open_in_memory()` 换掉连接再删目录。
- 回归：断言临时目录在 drop 后确实不存在（现有用例即可）。

### D4 workbuddy 契约测试在句柄未关时删除目录

- 位置：`providers/workbuddy/src/lib.rs`（`sessions_contract_check_accepts_v1_and_rejects_drift`）
- 成因：`db`（`ReadOnlyDb`）仍在作用域内时执行 `fs::remove_dir_all(root)`。
- 影响：Windows 上报 error 32「另一个程序正在使用此文件」。
- 修复：删除目录前显式 `drop(db)`。

### D5 clippy 在 Windows 上因 `unused_mut` 失败

- 位置：`crates/infrastructure/src/atomic_file.rs:12`
- 成因：`let mut builder` 只在 `#[cfg(unix)]` 分支里被 `mut` 使用。
- 影响：`clippy -D warnings` 失败 → CI 的 Windows job 直接红。
- 修复：把 `mut` 收进 `#[cfg(unix)]` 分支（或对 unix 分支单独取可变绑定）。

### D6 前端回归测试在 Windows 上无法创建目录符号链接

- 位置：`apps/desktop/tests/helpers.mjs:14`
- 成因：`symlinkSync(..., 'dir')` 在 Windows 上需要管理员权限或开发者模式，
  否则 `EPERM`。
- 影响：`pnpm test:desktop` 三个测试文件全部失败；
  CI 只在 Linux 跑该命令，所以一直没被发现。
- 修复：Windows 使用 `'junction'` 且目标必须是绝对路径
  （`junction` 不需要提权）；非 Windows 保持 `'dir'`。

---

## 2. 补齐 macOS 已有、Windows 缺失的能力

### D7 Antigravity：Windows 上被硬关闭，但本机已有真实样本

- 现状：`providers/antigravity/src/lib.rs` 的 `discover()` 走
  `#[cfg(target_os = "macos")]`，Windows 分支返回 `None` → provider 完全不可见。
- 本机样本：`%USERPROFILE%\.gemini\antigravity`，与 macOS **同构**：
  `conversation_summaries.db`(+`-wal`/`-shm`)、`conversations/<UUID>.pb`、
  `brain/<UUID>/...`。
- 证据：macOS 那条投影查询在 Windows 库上**原样可跑**，
  返回 12 行且 `status` 同为 `CASCADE_RUN_STATUS_IDLE`。
- 差异：Windows 样本的 `title` 为**空字符串**（macOS 为真实标题），
  适配器需把空标题降级为 `None`，不能显示成空标题。
- 任务：
  1. 按 `fixtures/README.md` 产出脱敏实样
     （`fixtures/antigravity/index/*`、`conversations/`、`brain/` 结构样本），
     记录 Windows 版本与采集日期；
  2. 更新 `docs/providers/antigravity.md` 增加 Windows 段落；
  3. `discover()` 改为 Windows/macOS 共用 `home/.gemini/antigravity`，
     但**先有样本再改代码**（仓库红线：不得先于调查建模）；
  4. 适配器空标题降级 + 一条 Windows 回归；
  5. 清理能力仍不启用（无 `docs/safety/antigravity-*` 恢复结论前保持 Unknown）。

### D8 Pi：Windows 未安装，禁止照抄 macOS 路径

- 现状：本机 `%USERPROFILE%\.pi` **不存在**，无任何 Windows 样本。
- 任务：先在 Windows 侧安装 `@earendil-works/pi-coding-agent` 并采集样本，
  再决定路径与格式；在此之前 `discover()` 保持 `None`（fail-closed）。

### D9 Claude Desktop：Windows 存在未被扫描的新目录树

- 现状：适配器只扫 `%LOCALAPPDATA%\Claude-3p\local-agent-mode-sessions`，
  本机还存在 `claude-code-sessions\`（新布局）**完全未被扫描**，
  属于静默覆盖缺口。
- 任务：按 `docs/providers/claude-desktop.md` 补一份 Windows 调查，
  决定是否纳入；纳入前在 Diagnostics 里显式报"未扫描的目录"，
  不允许静默少算。

### D10 Windows 专属数据位置尚未覆盖

- Phase 0 文档要求扫 `%APPDATA%`、`%LOCALAPPDATA%`、`%PROGRAMDATA%`，
  目前只有 `%LOCALAPPDATA%\Claude-3p` 被接入。
- 任务：逐 Provider 核对 `docs/providers/*.md` 的 Windows 段落，
  把"仅文档提及、代码未扫"的位置列成清单，
  逐个判定（纳入 / 明确排除并写进文档）。

### D11 进程签名只认 macOS 形态

- 现状：`workbuddy` 的签名 needle 是 `workbuddy.app/contents/`，
  在 Windows 上永远不匹配；`WorkBuddy.exe` 只靠 exe 名兜底。
- 任务：为每个 provider 补 Windows 进程形态（exe 名 / 应用路径），
  并为 `ProcessSignature` 增加 Windows 回归测试。

### D12 平台门禁挡住了本可复用的测试

- `apps/cli/tests/new_provider_sources.rs` 整体被
  `#![cfg(target_os = "macos")]` 关掉，但用例本身是跨平台的
  （它同时设置 `HOME` 与 `USERPROFILE`）。
- `crates/application/src/tests/live.rs::local_macos_readonly_scan_and_preview`
  断言 `host_platform() == Macos`。
- 任务：去掉 macOS 门禁 / 改为平台中立断言，让 Windows 直接复用这些回归。

---

## 3. Windows 侧缺少的原生验收（对应 macOS 的严格验收）

### D13 原生回收站往返（对应 `macos_acceptance.rs`）

- 新增 `crates/infrastructure/src/windows_acceptance.rs`：
  仅移动自己命名的临时样本到 **Windows 回收站**，
  校验 identity / 内容不变，再手动移回并复核。
- 回收站根：`%LOCALAPPDATA%\Microsoft\Windows\INetCache\Recycler` 或
  `SHEmptyRecycleBin` 语义；**不遍历、不清空**用户回收站。
- 必须覆盖：文件、嵌套目录、空格、中文、引号路径、只读属性文件。
- 门禁：`#[ignore]`，仅显式 `--ignored --exact` 运行；
  任何原生拒绝都算**失败**，不算跳过。

### D14 场景验收文档

- 新增 `docs/testing/windows-scenarios.md`，结构对齐
  `macos-scenarios.md`（场景 / 最近结果 / 验证入口 / 边界）。
- 首轮必须如实记录：Windows 特有失败（文件锁、回收站、权限、
  长路径、保留名、junction/云占位 reparse point），
  不从 Mac 结果推导通过。

### D15 桌面应用在 Windows 上的真实运行

- `pnpm tauri dev` 起 WebView2，核对真实数据（4 个安装 / 45 个会话）。
- 打包：NSIS + MSI 目标各出一次 debug 包，核对 `.ico` 与安装后启动。
- 记录 WebView2 缺失时的降级提示。

---

## 4. Phase 7 清理 Beta 在 Windows 上的落地顺序

前置：D1–D6 全部合入并通过 CI。

1. **WorkBuddy FileSet Beta**（依赖 D2）
   生命周期恢复为 `Inactive` / `Archived` 后，
   单元才会从 Blocked 变为可评估。Windows 上先只做**候选预览**，
   再做一次受控的真实清理（临时合成样本）。
2. **Claude Code FileSet Beta**
   相对独立，可在 WorkBuddy 之后并行；仍以临时合成样本验收。
3. **Codex ProviderOperation**
   macOS 侧合成样本试删返回 `failed to delete session`，
   成功路径与完整恢复在两端都**未验收**。
   Windows 需独立复现该失败并保持 Blocked，
   不得因为"Windows 上没试过"而默认放行。
4. **回收空间口径**
   移入回收站不等于释放空间；`reclaimed_bytes` 仍按 0 记，
   不以单元估算冒充实际释放，不自动清空回收站。

---

## 5. 建议执行顺序（按可交付价值排序）

| 批次 | 内容 | 出口条件 |
| --- | --- | --- |
| **B1** 修 CI 红灯 | D5 clippy、D4 workbuddy 测试、D3 `MutableSqliteDb` drop、D6 前端 junction | `cargo clippy -D warnings` + `cargo test --workspace` + `pnpm test:desktop` 在 Windows 全绿 |
| **B2** 修数据正确性 | D1 占用口径、D2 SQLite 读取档位 | WorkBuddy 41 个会话不再是 `unknown`；`allocated_bytes` 有值 |
| **B3** 平台门禁复用 | D12 去掉 macOS-only 门禁 | Windows 复用 CLI/Application 现有 provider 回归 |
| **B4** Provider 补齐 | D7 Antigravity（先样本）、D9 Claude Desktop 新目录树、D10 Windows 专属位置、D11 进程签名 | 每个 provider 在 Windows 有实测样本 + 文档 + 回归 |
| **B5** 原生验收 | D13 回收站往返、D14 场景文档、D15 桌面真机 | `docs/testing/windows-scenarios.md` 落库，回收站严格验收通过 |
| **B6** Phase 7 收口 | WorkBuddy → Claude Code 清理 Beta 候选；Codex 保持 Blocked | 各 Beta 有 `docs/safety/` 文档 + 受控真实清理记录 |

---

## 6. 需要在本轮同步更新的文档

- `docs/CHANGELOG.md`：Phase 状态表新增 Windows 维度；
  Phase 7 的"跨平台恢复/兼容性验收"从"未做"改为"Windows 进行中"。
- `docs/testing/windows-scenarios.md`：新增（B5 产出）。
- `docs/providers/{antigravity,pi,claude-desktop,workbuddy,codex,claude-code}.md`：
  补 Windows 段落；Pi 在拿到 Windows 样本前不写推测路径。
- `docs/safety/*.md`：每个在 Windows 上启用的清理类型都要有对应结论，
  否则保持 Blocked。
- 根 `CHANGELOG.md`：面向用户的变更（Windows 支持进度）按 Keep a Changelog 追加。

## 7. 明确不做的事

- 不因为 macOS 通过就推定 Windows 通过，反之亦然。
- 不为未调查的资源类型建模（Pi Windows 路径、Claude Desktop 新布局）。
- 不在 D2 修好之前把 WorkBuddy 清理单元从 Blocked 放行。
- 不开放 Codex native delete 与数据库替换。
- 不自动清空回收站、不承诺"已释放空间"。
