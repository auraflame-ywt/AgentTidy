# 项目进度 Changelog

本文件记录 AgentTidy 的开发阶段进度，帮助贡献者了解当前工作重点与
已知阻塞。它不是发布日志：面向用户的版本变更仍维护在仓库根目录的
[`CHANGELOG.md`](../CHANGELOG.md)。阶段定义以 [`Start.md`](../Start.md#19-开发顺序)
为准。

## 当前概览

| 阶段 | 状态 | 说明 |
| --- | --- | --- |
| Phase 0：Product & Provider Investigation | 已完成 | 三个首批 Provider 的 macOS / Windows 样本调研与脱敏 fixtures 已具备。 |
| Phase 1：Read-only Core | 已完成 | 只读领域模型、能力模型、快照和静态 Provider Registry 已实现。 |
| Phase 2：Infrastructure | 已完成 | 安全文件系统、只读 SQLite、JSONL、空间统计、进程信号与回收站封装已实现。 |
| Phase 3：三个 Read-only Provider | 已完成 | Codex、Claude Code（CLI/Desktop）和 WorkBuddy 均已具备安全降级的只读扫描。 |
| Phase 4：CLI Validation | 已完成（只读初版） | CLI 已提供版本化 JSON 契约、文本诊断、真实扫描与 12 个端到端 golden；清理与候选预览保留给 Phase 6/7。 |
| Phase 5：GUI Read-only Experience | 已完成（只读初版） | Tauri 2 IPC + React 三视图（Overview / Sessions / Diagnostics）已接入 Application API；清理与候选预览保留给 Phase 6/7。 |
| Phase 6：Cleanup Model | 已完成（framework-only 初版） | 清理领域模型、策略、重新验证、审计日志、IPC 与 GUI Review 视图已落地；三个 provider 均未产出 Cleanup Unit，真实清理归 Phase 7。 |
| Phase 7：首个 Provider Cleanup Beta | 进行中（代码切片全部落地） | 三个 provider 的清理切片均已落地：Claude Code transcript FileSet Beta、Codex archived-session `ProviderOperation` Beta（native `codex delete` 优先 + §12.4 自实现兜底）、WorkBuddy sessions v1 契约冻结 + FileSet Beta；Windows 锁定文件 `skipped` 分类已实现。剩余：小范围真实用户 Beta 与 Trash/Recovery/Compatibility 跨平台验证。 |
| Phase 8：逐 Provider 扩展 | 未开始 | 按 Provider 分别通过安全门槛后推进。 |

## 已完成里程碑

### Phase 0：Product & Provider Investigation

- 完成 WorkBuddy、Claude Code 和 Codex 的真实样本调研文档，以及 Windows
  路径矩阵和 Claude Desktop 的补充调研。
- 添加按 Provider 划分的脱敏目录结构与会话 JSONL fixtures。
- 明确 v0.1 的首批范围为 macOS 与 Windows 上的三个 Provider；默认工作区
  只报告、不清理。

### Phase 1：Read-only Core

- 实现安装信息、Session、Resource、大小置信度、能力状态、扫描快照和
  静态 Provider Registry 的只读领域模型。
- 实现 `AgentProviderAdapter` 只读契约；清理相关接口有意延后至 Phase 6。
- 采用闭集 Provider 标识与缺失能力默认 Unsupported 的 fail-closed 规则。

### Phase 2：Infrastructure

- 实现路径边界与保留名称检查、禁止跟随链接的目录遍历、硬链接去重与
  分配空间统计。
- 实现逐行 JSONL 解析、只读 SQLite（含 WAL / 临时副本回退）、进程存在性
  信号和系统回收站封装。
- 平台特定实现限定在 infrastructure 层，符合架构边界。

### Phase 5：GUI Read-only Experience

- Tauri 2 IPC 桥接：`apps/desktop/src-tauri/src/commands.rs` 注册 `doctor`、
  `scan`、`detect` 三个命令，转发到 `agenttidy-application`，薄壳层无业务逻辑。
  `Result<T, String>` 作为 IPC 错误形态；`capabilities/default.json` 是最小
  占位（应用命令经 `tauri::generate_handler!` 自动放行）。
- React 三视图（Start.md §6.1 / §6.2 / §6.4）：Overview 卡片展示每个安装的
  capability、sessions / resources / total size / workspace footprint 与问题计数；
  Sessions 表格列出所有可识别 Session 及其 lifecycle / cwd / 大小；
  Diagnostics 折叠面板展示 inspection.problems / readable_roots /
  schema_versions / journal_modes / data_roots。
- TS 类型在 `apps/desktop/src/types.ts` 手写对齐 Rust serde shape；`pnpm build:desktop`
  的 `tsc -b` 步骤拦截 IPC 字段重命名与类型不匹配。自动 mount 扫描 + 手动 Refresh
  按钮；空安装路由到独立 `Empty` 面板，与 CLI 的空 envelope 契约对齐。
- §6.3 Review & Tidy 在 Phase 6 交付（见下方 Phase 6 里程碑）；Phase 5 收官时
  GUI 仍不暴露执行入口。

### Phase 6：Cleanup Model（framework-only 初版）

- 领域模型 `crates/core/src/cleanup.rs`：三级 `RiskLevel`（§7）、`CleanupUnit`
  / `CleanupPlan` / `CleanupItem`（§9.5 / §12）、12 个 §12.2 重新验证
  precondition kind、tagged `CleanupLocator` 与 §16.3 `CleanupEvent`
  （含 `schema_version: agenttidy.audit.v1` + `platform`）。
- Provider 契约：`AgentProviderAdapter` 新增 `build_cleanup_units` /
  `validate_cleanup_unit`，默认实现返回空。**三个 provider 的 override 均显式
  返回空** —— Codex workspace 是 user-mixed（§2.3）、session JSONL 会孤儿化
  `state_5.sqlite.threads`；Claude Code FileSet 与 WorkBuddy（`sessions` 表契约
  未冻结）均排 Phase 7，各自在 doc 注释中说明门槛。
- Application 层：纯函数 `cleanup_policy_evaluate(unit, ctx)`（§11，注入
  `now_ms` 保证确定性）、`cleanup_plan_from_snapshots`（SHA-256 plan
  fingerprint + 5 分钟 TTL）、`cleanup_revalidate`（§12.2 逐项触发器 + 报告
  哪一条失效）、`cleanup_execute`（fingerprint 不匹配即拒绝；幂等
  AlreadyGone ⇒ Skipped；每项写审计日志）。旧
  `workspace_cleanup_candidates_from_snapshots` 保留为迁移期投影，既有 4 个
  测试不回归；新增 10 个测试覆盖三个风险等级、空 plan、fingerprint 拒绝与
  六类重新验证触发器。
- Infrastructure：`paths::home_dir()` 共享 helper；`TrashOutcome`
  （Removed / AlreadyGone / PlatformError）替换二元返回；`audit_log` 模块
  （`$HOME/.agenttidy/operations.jsonl`，append-only + fsync，JSONL roundtrip
  测试）。
- IPC + GUI：`agenttidy.gui.v1` envelope 的三个 Tauri 命令
  （`cleanup_preview` / `cleanup_revalidate` / `cleanup_execute`）对应两步确认
  流程的三个阶段；前端 `Tidy` tab 按 Mac cleaner 视觉锚点实现 hero stat、
  按 provider 分组行、Recommended / Caution / Off-limits 徽章、stale 行
  “changed — rescan” 标记与需键入 CONFIRM 的第二次确认 modal。
  空 plan 显示 Phase 7 rollout banner。
- `docs/safety/workspace-cleanup.md` 与 §11/§12 对齐：三级风险词汇表、
  12 条重新验证触发器、审计日志 schema、`Trash` vs `ProviderOperation`
  边界（DB 行永不标签为 Move to Trash）、operation guard 推迟至 Phase 7
  的 threat-model 备注。`docs/safety/README.md` 增加资源类型状态表。
- CLI 保持只读：无 `clean` 子命令；12 个 CLI golden 不变。

### GUI 双语（zh/en）

- 手写轻量 i18n（`apps/desktop/src/i18n/`）：`en.ts` 为 source of truth，
  `zh.ts` 以 `Record<TranslationKey, string>` 约束——缺 key / 多 key 都会让
  `tsc -b` 失败，`pnpm build:desktop` 即完整性门禁（双向均已自证）。首次启动
  按 `navigator.language` 检测，header 提供 EN / 中文 切换并持久化到
  `localStorage`，同步 `<html lang>`。
- 翻译范围三层：前端静态文案与枚举标签全译（status / capability /
  lifecycle / risk / confidence / severity / outcome）；后端 `ScanProblem`
  按稳定 code / 路径后缀映射到中文、未知 code 回退英文原文；cleanup
  reasons 与 anyhow 错误保持英文（Phase 7 随安全文档一起映射）。
  `format.ts` 收编四处重复的 `humanBytes`，三处时间戳改为按语言本地化格式。
  CLI 输出（`agenttidy.cli.v1` 自动化契约）永远英文。

### Phase 3：三个 Read-only Provider（补记）

- Codex 已接入 `AgentProviderAdapter`：发现共享的 CLI / Desktop 状态根，检查
  读取权限和运行信号，并从 rollout JSONL 的首条 `session_meta` 生成只读 Session
  与 transcript Resource；不会读取对话正文。
- Codex 以只读 `state_5.sqlite.threads` 映射补齐标题、时间、工作目录和归档
  状态；索引不可读、完整性检查失败或归档位置不一致时均记录诊断，冲突 Session
  降级为 Unknown，绝不写入数据库。
- 无法识别的 rollout 默认只报告问题，只有在 `include_unknown` 时才以 Unknown
  资源展示。下一步是补齐 Provider fixture / contract 覆盖，并以相同安全边界
  实现 WorkBuddy 与 Claude Desktop 安装。
- Claude Code CLI 已接入 `AgentProviderAdapter`：仅扫描已验证的
  `~/.claude/projects/<cwd-slug>/<session-id>.jsonl` 布局；主 transcript 与同名
  side directory 作为同一 Session 资源计量，嵌套 subagent 不会被独立解析或重复
  计数。解析仅保留会话元数据，不保留消息正文。
- Claude Code Windows Desktop 的 VM 安装、凭证与镜像副本尚未接入，维持不扫描、
  不触碰的边界，直至其镜像关系与 capability 规则获得单独的契约测试。
- WorkBuddy 已接入 `AgentProviderAdapter`：扫描已验证的 transcript、同名
  tool-results directory、`.meta.json` 和 `.file-rollback.ndjson` 文件集；数据库
  只用于健康与 journal mode 检查。因尚未冻结 `sessions` 表契约，WorkBuddy Session
  的生命周期保守标为 Unknown。
- Claude Code Desktop 在 Windows 上发现为 `claude-code:desktop`，仅扫描
  `local-agent-mode-sessions` 中嵌入的 `.claude/projects` transcript 镜像；不读取
  VM 镜像、凭证、审计日志或用户输出目录。

### Phase 4：CLI Validation（只读初版）

- 建立 Application API 的静态 Provider Registry，统一调度 Detect / Inspect / Scan。
- 将 CLI 的 `doctor`、`scan`、`sessions` 占位命令接到该 API：显示安装、能力、
  Session/Resource/空间汇总与诊断；`clean` 明确拒绝，保持只读。
- `doctor`、`scan` 与 `sessions` 均支持带
  `schema_version`、`command`、`mode: "read-only"` 的 JSON 输出契约
  `agenttidy.cli.v1`；文本输出则面向人工诊断。
- Application 层以无副作用的纯函数应用工作空间候选门槛，并以单元测试锁定：精确
  单 Session + 文件系统安全事实才能预览为 eligible；WorkBuddy 在自动化引用契约
  未验证前始终 blocked。该能力只供未来 GUI 的候选选择与确认流程使用，不暴露给 CLI。
- 已在开发机验证真实扫描结果，并按真实 provider fixtures 增加端到端 CLI
  golden 基线：`apps/cli/tests/golden/` 下的 12 个提交快照（3 provider ×
  3 command + 3 empty-home 回归）锁住 `agenttidy.cli.v1` envelope 与降级
  行为（Codex 缺 SQLite、WorkBuddy 缺 db 时各自的警告路径与 workspace
  resource 字段）。绝对路径、完成时间戳、`agent_running`、`lifecycle.state`
  与 `platform` 字段以占位符归一化，单套金色可同时用于 macOS 与 Windows
  CI。新增 provider 或 schema 字段变更会立即破坏金色，需要同步更新。
- 默认工作空间与常见 AppData/缓存路径的只读统计将作为本阶段后续工作；它们与
  Session 占用分开显示，默认仅报告。
- Codex `Documents/Codex` 与 WorkBuddy `WorkBuddy` 默认工作空间已纳入扫描快照和
  CLI 汇总，作为 `Workspace / User / report-only` 资源独立统计；不会归入 Session
  或可回收空间。

### Phase 7：首个 Provider Cleanup Beta（进行中）

已完成（第一阶段：Claude Code transcript FileSet Beta）：

- 资源类型安全文档 `docs/safety/session-transcript-fileset.md`（先文档后
  启用，满足 AGENTS.md 红线）：单元构成、永不入单元的清单、Trash/恢复
  边界、7 条 §12.2 前置触发器、§16.1 路径安全、并发与审计约定。
- Claude Code `build_cleanup_units`：每个已识别、独占的 Session 资源产出
  一个 `FileSet` 单元（transcript + 同名 side 目录），transcript 携带
  identity/size/mtime 指纹；无法指纹化的资源保持 report-only。
- `validate_cleanup_unit(unit, installation)`（契约签名变更）：§16.1 校验
  ——canonicalize 后必须落在 installation data roots 内（symlink 逃逸拒绝）、
  成员共享同一 project 目录、布局必须是已验证的 `projects/<slug>/`；
  计划构建时调用，校验失败使 plan 构建失败（fail-closed）。
- Application 策略落地 §11.1/§11.2 生命周期规则：Active/Unknown →
  Blocked；`Archived + inactive > 30d + exclusive + recoverable` →
  LowRisk（唯一晋升路径）；Inactive / 近期 Archived → ReviewRequired；
  无快照链接、非独占、带依赖 → Blocked。
- 执行器：§12.1 跨进程 Operation guard（`$HOME/.agenttidy/operations.lock`，
  阈值 10 分钟的 stale-lock 接管保障崩溃恢复）；`move_paths_to_trash`
  多路径移动 + §12.3 部分失败诚实聚合（moved 计数 + 原文错误，永不误报
  成功/丢失）；审计日志记录完整 `file-set` locator 与
  `estimated_reclaimable_bytes`。
- `CleanupUnit`/`CleanupItem` 新增 typed `CleanupLocator` 与
  `estimated_reclaimable_bytes`，GUI 字节统计改用单元估算（FileSet 含
  side 目录），行内显示 `+N` 成员数。
- 测试：策略 10 项（三个风险等级 + 各阻断规则）、聚合 3 项、FileSet
  重验证、端到端执行（含审计行 + 幂等重跑）、Operation guard 4 项、
  provider 契约 5 项（含 symlink 逃逸与布局拒绝）。
- §6.3 Review & Tidy 体验闭环（GUI）：行级展示补齐操作方式
  （Move to Trash / Agent 专属操作）、可恢复性（系统回收站可恢复 / 不可）
  与 `file-set` 成员路径展开列表；选择规则加固——blocked 在状态层拒绝
  选择（不只靠 disabled checkbox）、stale（changed）行在重新验证时自动
  移出选择、执行后清空选择并要求重新扫描；计划过期（5 分钟 TTL）在
  两个 CTA 与点击时实时拦截并显示“计划已过期，请重新扫描”。
- TTL 后端强化（fail-closed 修复）：`cleanup_revalidate` 对过期计划的
  每一项报 `skipped`，`cleanup_execute` 直接拒绝执行，堵住裸 IPC 绕过
  GUI 过期门禁的口子（§12.2）。

### Phase 7 后续切片（2026-09-30）

- **Codex `ProviderOperation` Beta**（`docs/safety/provider-operation.md`）：
  `AgentProviderAdapter` 新增 `execute_provider_operation` trait 方法
  （默认 `Failed("not supported")`，不破坏未接入 provider）；Application
  层 `cleanup_execute` 的 `ProviderOperation` 分支从 Phase 6 stub 改为
  dispatch 到 adapter。Codex 侧：`build_cleanup_units` 对 archived +
  exclusive session 产出 `DbRow` locator 单元（`state_5.sqlite` 缺失时
  fail-closed 不发单元）；`validate_cleanup_unit` 校验 database 在
  data roots 内；执行路径优先 `codex delete <id>`（protocol
  investigation 关闭了 Open Question，codex-cli 0.152.0 验证），CLI
  不可用时走 §12.4 自实现：`backup_sqlite_with_sidecars`（VACUUM INTO +
  quick_check 门）→ `MutableSqliteDb::open_mutable_on_temp_copy`（类型
  系统保证永不直接打开 agent 源 DB 写）→ BEGIN IMMEDIATE 事务删除
  `threads` / `thread_attachments` / `thread_spawn_edges` →
  wal_checkpoint + quick_check → 原子 rename 回源文件；备份路径写入
  audit `reason`。`inspect()` 现在输出 `state_5` 的 `user_version` +
  journal mode（`SchemaUnchanged` 依赖）。
- **WorkBuddy sessions v1 契约冻结 + FileSet Beta**
  （`docs/safety/workbuddy-session-fileset.md`、
  `fixtures/workbuddy/db/sessions-schema.json`）：scan 从 `sessions` 表
  推导 lifecycle（`deleted_at NOT NULL` → Archived；终态 status →
  Inactive；其余 Unknown fail-closed）；`automations.cwds` 读出后挂到
  workspace 资源的 `automation_cwd_refs` metadata；契约漂移
  （列集不匹配 v1）时跳过推断。`build_cleanup_units` 对 Inactive/
  Archived session 产出 FileSet 单元（Trash 动作，DB 行不改动）；
  `validate_cleanup_unit` 复用 infrastructure 的
  `validate_transcript_fileset_paths`（§16.1 通用路径安全，从 Claude
  Code 抽取）+ DB 行存在性交叉校验。
- **Windows 锁定文件 `skipped` vs `failed` 分类**：`TrashOutcome` 新增
  `Locked` 变体（`ERROR_SHARING_VIOLATION`/`ERROR_LOCK_VIOLATION` 及
  HRESULT 形式）；聚合规则——仅锁定且无成员移动 → `Skipped`（提示关闭
  占用进程后重试）；锁定出现在部分成员已移动之后 → 诚实 `Failed`
  （§12.3）；其他平台错误 → `Failed`（原文错误）。
- 测试：Codex provider 7 项（含 §12.4 端到端集成）、WorkBuddy
  provider 6 项（契约冻结 + lifecycle 推断 + 单元构建 + 行缺失拒绝）、
  infrastructure（backup 3 项 + MutableSqliteDb 2 项 + fileset helper
  6 项 + lock 分类 1 项）、application（dispatch 契约 + 锁定聚合）。

## 下一步

### Phase 7 收尾（剩余验收项）

- 小范围真实用户 Beta 与 Trash/Recovery/Compatibility 跨平台验证
  （Start.md §19 Phase 7 验收项；macOS 已绿，Windows 需 CI + 真机）。

### Phase 8：逐 Provider 扩展（待启动）

每个 Provider 独立通过安全门槛后开放 Cleanup（Start.md §19）；
资源级启用流程沿用 Phase 7 的模板：先 `docs/safety/` 文档，再
`build_cleanup_units` / `validate_cleanup_unit`，最后策略 per-kind 提权。

### 验证前置问题

- macOS 无 GUI / Finder 服务时，回收站功能会以 `TrashError::Platform` 失败；
  单元测试会明确跳过原生回收站断言，生产调用仍保持 fail-closed。其余 Rust
  测试与 Clippy 在当前环境通过。
- 前端构建在受限会话下的不确定状态已解决：`pnpm build:desktop`（`tsc -b`
  与 `vite build`）与 `cargo build -p agenttidy-desktop` 在当前会话下均能
  干净通过；前端 build 不再是 Phase 5 验证的阻塞项。

## 维护约定

- 完成或范围变化时，更新对应 Phase 的状态、关键产出和“当前工作”内容。
- 仅当阶段验收条件已满足时标记为“已完成”；已知阻塞和跨平台差异必须保留。
- 发布面向用户的功能或修复时，同时更新根目录 `CHANGELOG.md`。
