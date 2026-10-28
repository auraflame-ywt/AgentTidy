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
| Phase 5：GUI Read-only Experience | 已完成，Mac 体验已优化 | 总览、会话搜索/筛选/详情、诊断已接入共享 API；已补齐占用排行、品牌图标、启动页与中英文展示。 |
| Phase 6：Cleanup Model | 已完成，执行框架已加固 | 计划、TTL、完整证据重验、跨进程锁、审计、IPC 与两次确认已落地；具体资源清理资格及恢复验收归 Phase 7。 |
| Phase 7：首个 Provider Cleanup Beta | 进行中（Mac 与 Windows 均已受控验收） | Claude Code / WorkBuddy FileSet Beta 已实现；Codex 仅保留 Blocked 候选，native delete 与 DB 替换均关闭。Windows 上已完成受控合成样本的"发现 → 计划 → 重验 → 移入回收站 → 手动恢复"完整闭环验收。剩余：真实用户 Beta、发行包与签名体验。 |
| Phase 8：逐 Provider 扩展 | 已开始（Windows 只读已对齐） | Antigravity 已在 Windows 接入（样本同构、空标题降级、盘符 URI 均有回归）；Pi 在 Windows 保持关闭（本机无安装、无样本）。清理/归档/恢复仍等待各自安全门槛。 |

## 2026-10-28 Windows 对齐 macOS：执行结果（最新状态）

本轮把 Windows 从"CI 红灯 + 静默丢数据"推进到"与 macOS 同门禁、同语义"，
并完成 Phase 7 清理 Beta 的 Windows 受控验收。**四道门禁在本机全绿**：

| 门禁 | 2026-10-24 基线 | 2026-10-28 结果 |
| --- | --- | --- |
| `cargo fmt --all --check` | 通过 | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | **失败**（`unused_mut`） | **通过** |
| `cargo test --workspace` | **失败**（临时目录泄漏、句柄未关） | **通过：185 项、0 失败、2 项默认忽略** |
| `pnpm build:desktop` | 通过 | 通过 |
| `pnpm test:desktop` | **失败**（symlink EPERM） | **通过：19 项** |

修掉的 Windows 阻塞缺陷：`atomic_file.rs` 的 `mut` 收进 unix 分支；
`MutableSqliteDb::drop` 先换内存连接再删目录；WorkBuddy 契约测试删目录前
显式 `drop(db)`；前端回归改用 `junction`（免提权）。

修掉的**跨平台**数据正确性缺陷（两端同时受影响）：

- §8 占用口径：`allocated_bytes` 由 `None` 起算导致永远为 `None`，所有平台
  的"分配大小"从未生效。本机修前 45/47 个会话类资源为 null，修后 0 个。
  CLI golden 基线随之改用既有的 `<ALLOCATED>` 占位符。
- 只读 SQLite 读取档位：`mode=ro` 打开缺 `-shm` 的 WAL 库**惰性成功**，
  私有副本降级从不触发。现在打开后先跑一次真实查询验证可读，并在打开前
  校验 SQLite 文件头，避免把非数据库文件读成空库。
- 独占归属：`is_single_link_file` 在所有非 unix 平台恒为 `false`，Windows 上
  **没有任何会话能被证明独占**，因而永远产不出清理单元。现在读取
  `BY_HANDLE_FILE_INFORMATION.nNumberOfLinks`（Windows 的 `st_nlink`）。

Mac 同步（消除平台门禁）：

- Antigravity 在 Windows 接入：状态根、索引与三棵已验证目录与 macOS 同构，
  投影查询原样可跑；12 条索引、760 个资源。两处真实差异已有回归——
  Windows 样本的 `title` 全为空（降级为"无标题"，不显示空白标题），
  `workspace_uris` 带盘符（`file:///c%3A/...` → `c:\...`）。
- Pi 在 Windows **保持关闭**：本机没有安装、没有任何样本，按红线"先有样本
  再改代码"不得照抄 macOS 路径。
- 去掉 macOS-only 门禁：CLI 新 Provider 端到端用例（Antigravity 在 Windows
  跑，Pi 忽略）与 Application 本机只读探针现在两端共用。

Phase 7 受控验收（Windows，本机）：

- **原生回收站往返**：Recycle Bin 的 `$I`/`$R` 结构与 macOS Trash 不同
  （原名不保留，原路径存在 `$I` 元数据里），新增
  `crates/infrastructure/src/windows_acceptance.rs` 证明文件、嵌套目录、
  空格、中文、引号路径的真实往返，identity 与字节内容不变。
- **完整 Beta 闭环**：新增 `crates/application/src/tests/cleanup_acceptance.rs`，
  在隔离 home 下铺出真实布局与真实 SQLite 的合成 WorkBuddy 安装，走完
  发现 → 计划 → 重验 → 移入真实回收站 → 幂等重跑 → 手动恢复并逐字节核对。
  全程只动自己唯一命名的临时样本。
- 本机只读快照：5 个安装、57 个会话（Claude Code CLI 1 + Desktop 1、
  Codex 2、WorkBuddy 41、Antigravity 12）。WorkBuddy 因此在 Windows 上
  产出 41 个可评估单元（修复前为 0）。

场景记录见 [`testing/windows-scenarios.md`](./testing/windows-scenarios.md)；
完整计划与批次划分见 [`windows-parity-plan.md`](./windows-parity-plan.md)。

## 2026-10-24 Windows 对齐 macOS 计划（历史基线）

- 在 Windows 开发机实测三道门禁并留存证据：`cargo fmt --all --check` 通过；
  `cargo clippy --workspace --all-targets -- -D warnings` **失败**（`atomic_file.rs`
  在非 unix 分支的 `unused_mut`）；`cargo test --workspace` **失败**
  （`MutableSqliteDb::drop` 未先关连接导致 Windows 删不掉临时目录；
  WorkBuddy 契约测试在句柄未关时删目录）；`pnpm build:desktop` 通过；
  `pnpm test:desktop` **失败**（前端回归用目录符号链接，Windows 需改 junction）。
  CI 的 Windows job 目前是红的，Linux 独占的前端门禁掩盖了同类问题。
- 只读 CLI 在本机可用：检出 `claude-code:cli`、`claude-code:desktop`、
  `codex:default`、`workbuddy:default`，共 45 个会话。Antigravity 与 Pi 在
  Windows 上被 `cfg(target_os = "macos")` 硬关闭。
- 定位到两个跨平台数据正确性缺陷：占用聚合的 `allocated_bytes` 因初值为
  `None` 而恒为 `None`（§8 分配大小从未生效）；只读 SQLite 的 `mode=ro` 打开
  在 WAL 库缺 `-shm` 时**惰性成功**，导致不落到私有副本降级，
  WorkBuddy 生命周期推断被跳过（41 个会话全部 `unknown`，
  Phase 7 的 WorkBuddy FileSet Beta 在 Windows 上产出 0 个可执行单元）。
- Windows 侧本机已有 Antigravity 真实样本（`%USERPROFILE%\.gemini\antigravity`，
  与 macOS 同构，投影查询原样可跑），Pi 未安装。Claude Desktop 存在未被扫描的
  `claude-code-sessions\` 新目录树。
- 完整计划、证据与批次划分见 [`windows-parity-plan.md`](./windows-parity-plan.md)。
## 2026-10-21 Antigravity / Pi 扩展（最新状态）

- 基于本机真实样本新增两个静态 Provider，贯通 Rust Application API、CLI 与桌面。
  macOS 默认路径检测；Pi 支持 agent/session 目录环境变量。
- Antigravity 只读取索引元数据并测量 `.pb` / brain 产物，支持单项目路径映射与
  多工作区详情；Pi 只读取有大小上限的 v3 header，不解析后续消息正文。
- 新来源统一标签与筛选，补齐中英文来源名称和空状态说明。原有清理能力未扩展。
- 两者均保持生命周期/归属 Unknown，不创建 cleanup unit，不启用归档、删除或恢复。
  Windows 尚未调查与验收。Phase 7 现有恢复/用户验收阻塞不因扩展而关闭。
- 本机只读扫描：Antigravity 50 条会话（4 条索引缺少对应可读正文，另有未索引文件
  提示）、Pi 3 条会话；未执行真实数据清理。
- 实样来源、格式边界和测试说明见 `docs/providers/antigravity.md` 与
  `docs/providers/pi.md`，脱敏样本位于 `fixtures/antigravity`、`fixtures/pi`。
- 验证：Rust 工作区 179 项通过、2 项原有测试默认忽略；fmt、clippy（warnings
  视为错误）、TypeScript/Vite 构建和 19 项前端测试通过。新增来源 CLI 端到端通过；
  已生成包含两类来源的 macOS Debug `.app`。

## 2026-10-20 Mac 检查与品牌升级总结

本轮范围为 **Mac 本机检查、修复、使用路径优化和品牌升级**。默认只读、
Trash-first、Fail-closed 的边界未放宽；没有清理真实会话，也没有执行
Windows 真机验收。Phase 7 继续保持进行中。

### 安全与执行修复

- Codex 仅保留归档候选预览，始终 Blocked；直接 Provider 调用也拒绝执行。
  native delete 和直接 DB 替换均关闭，避免将 SQLite 备份误当作完整会话恢复。
- 同一物理数据库多选在任何修改前整批拒绝，覆盖 canonical 别名与硬链接；
  单选不被未选中的行干扰。尚未实现协调式同库批量删除。
- Codex CLI 缺失、探测失败、版本不符提前显示在预览与诊断；探测限定目标
  CODEX_HOME，并设 3 秒超时与子进程取消，受支持版本也不能越过恢复门槛。
- 经真实本机字段核对，将 Codex 秒级时间转换为领域毫秒，修正 1970 年日期、
  排序及时间筛选；负数/溢出保持未知。
- 完整证据、OS advisory lock、操作前审计、部分失败及 TTL 门禁继续生效。
  原生 Trash 恢复使用临时样本验证，不将移入 Trash 的估算量冒充实际释放空间。

### 产品与页面

- 总览先呈现已识别占用、Agent 分布、诊断及最大会话入口；检查与扫描的诊断
  合并去重，逻辑大小与可审查估算分开展示。
- 会话列表保留搜索、Agent/生命周期/时间筛选与分页；新增原生详情侧栏，
  展示项目、关联资源和处理条件；仅使用现有元数据，不读取正文猜标题。
- 审查页优先展示会话/项目身份与生命周期理由，路径、计划 ID 和完整原因
  按需展开。身份关联必须唯一，不从 unit ID 或共享数据库推断会话归属。
- 本次处理回执只统计提交项；保留原位置、部分失败和手动恢复说明。
  响应缺失时明确提示结果需核对，扫描刷新后仍保留本次身份；没有新增
  跨重启的执行历史、自动恢复或虚假 Undo。

### 品牌与启动

- 新图标采用深石墨、银白折叠归档结构与翡翠绿点睛，统一 ICNS、ICO、桌面
  PNG、favicon、工作台标识和启动页；移动端资源未更换。
- 启动页采用同一视觉语言，显示真实扫描状态，可后台继续；就绪后短过渡
  进入工作台，支持减弱动态效果，失败时可重试/查看诊断。
- 点击工作台左上图标可重新打开品牌页，不触发新扫描、不重置清理状态。
- 源图、导出脚本与最终 imagegen 提示词见
  [品牌资源说明](../apps/desktop/assets/BRAND.md)。

### 代码与验证

- 分离只读扫描、清理和启动展示的状态模块；预览延迟至首次进入整理页，
  导航保留筛选且不取消正在进行的执行；未新增前端运行依赖。
- Rust 最近一次全工作区检查：**165 项通过、2 项 opt-in 默认忽略**；
  fmt、Clippy（warnings denied）通过。
- 品牌升级后前端：**17 项回归通过**，TypeScript/Vite 构建与 Tauri debug
  `.app` 打包通过；图标透明通道、导出尺寸及包内 ICNS 一致性已核对。
- 本机只读快照：3 个安装、354 个会话（Claude Code 26、Codex 19、WorkBuddy
  309）。数量及运行状态随真实 Agent 活动变化，不作为固定验收值。
- 本机只读探测与严格 Trash 临时样本恢复在本轮较早阶段分别显式通过，
  不计入 165 项常规测试；后续品牌改动未重新运行严格 Trash 测试。
- Mac 原生界面已核对总览跳转、侧栏、审查阻断、Codex 日期、品牌页与后台
  继续；错误/响应缺失等组合由自动化覆盖，不冒充真实删除端到端验收。
- 本机 pnpm 启动器存在签名校验/获取问题，使用已安装 Node 直接执行同等
  TypeScript、Vite、测试及 Tauri 入口；未关闭签名验证或改 lockfile 绕过。

### 尚未完成

1. 受控 Provider 样本的完整 GUI 点击 → 执行 → 恢复 → 原 Agent 兼容性验收，
   随后开展小范围真实用户 Beta。
2. Codex 原生成功路径与数据库、transcript、索引、附件的完整恢复验证；
   通过前维持 Blocked。同库协调批处理另行设计与验证。
3. Mac 跨卷/外接盘恢复、TCC 权限场景、签名/公证发行包体验。
4. Windows CI/真机清理与恢复单独推进；本轮结果不推导其通过。
5. 后续工程整理：策略下沉 core、Application 模块化、Rust/TS 契约自动同步、
   请求内只读事实复用。以上仍是待办，不属于本轮已完成项。

详细场景、运行入口和边界见 [Mac 验收记录](./testing/macos-scenarios.md)。

## 已完成里程碑（历史记录）

以下按当时阶段保留实现演进与测试基线，不代表当前启用状态。尤其是旧锁
接管机制、Codex native delete/数据库替换原型及早期测试数量，均以本页
“本轮总结（最新状态）”和安全文档为准。

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
- TS 类型在 `apps/desktop/src/types.ts` 手写对齐 Rust serde shape；`tsc -b`
  检查前端类型一致性，不能自动发现 Rust 字段重命名，仍需跨边界契约验证。自动 mount 扫描 + 手动 Refresh
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
  当时采用 stale-lock 接管，后已由 OS advisory lock 取代，禁止按年龄抢锁）；`move_paths_to_trash`
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

- **Codex `ProviderOperation` Beta 原型（已停用）**
  （当前状态见 `docs/safety/provider-operation.md`；以下原型路径已移除）：
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

### Phase 7 执行链路加固（2026-10-20）

- 去掉 Application 实例内安装缓存，执行时重新发现安装，修复独立 Tauri IPC
  请求间 Codex ProviderOperation 找不到 installation 的问题。
- Codex 指纹的 identity / mtime / size 全部来自数据库；执行前重新扫描、
  构建单元、验证 Provider 安全约束并重算策略，活跃/Unknown/非独占/风险
  变化或成员变化均跳过。Schema SQL / user_version / journal 与预览比较。
- 预览携带 canonical paths 和递归成员指纹，覆盖 side directory、关联
  rollout 与已有 DB sidecars；完整 item payload + expiry 纳入确认指纹。
  操作锁先于重验证获取，每项执行前再验证，TTL 在重验证后再次检查。
- SQLite 备份改为对只读源执行 VACUUM INTO，WAL 合并进独立快照，不再使用
  文件拷贝兜底或覆盖已有备份；新增 live-WAL / recovery-point 回归测试。
- Codex 仅接受调研版本 CLI，显式设置目标 CODEX_HOME，执行前备份 state 与
  存在的 history DB。旧替换兜底未证明跨库 / WAL / Windows 恢复安全，已移除；
  CLI 缺失、版本不符或失败时 fail-closed，不报告虚假的回收字节。
- 增加独立实例 JSON 往返执行、审计/幂等、会话重激活、Schema/附件变化、
  Provider 校验拒绝及 payload 修改回归；修复回收站失败场景中的测试假设。

- 本机验证：`cargo fmt --all --check`、Clippy（warnings-as-errors）和
  `cargo test --workspace` 全部通过（153 项）；前端直接执行 `tsc -b` +
  `vite build` 通过。`pnpm build:desktop` 的启动器在获取指定 pnpm 版本时
  registry signature verification 报 fetch failed，未将该命令记作通过。
  本轮未执行真实用户数据清理，也未完成 Windows / 原生恢复验收。

### 本机 Mac 场景补齐（2026-10-20）

- 新增 [`testing/macos-scenarios.md`](./testing/macos-scenarios.md) 场景矩阵，
  区分普通自动化、真实元数据只读检查、严格原生恢复及未验收项。
- 修复 WorkBuddy 5.8.3 Electron 主进程漏报；本机三 Provider 当前均报告运行中，
  实际 `.app` 的 337 个单元均阻断选择。353 个会话正常展示、可搜索和筛选。
- NSFileManager 原生 Trash + 临时文件 / 嵌套目录恢复严格通过，覆盖中文、空格、
  引号路径；不再依赖 Finder 脚本。Trash 移动 actual reclaimed_bytes=0。
- Operation guard 改为 OS advisory lock，覆盖子进程竞争 / crash release / 老锁
  不抢占；审计改为操作前打开 sink + fsync intent/result，半行恢复与权限失败有回归。
- 无法验证的单元保留为 Blocked 行，不让整个预览失败，也不计入可处理字节。
- GUI 增加会话搜索、Agent / 生命周期 / 时间过滤、排序与分页；清理期间禁用重复
  请求与选择，过期弹窗自动失效，执行尝试后强制重新预览；常见阻断理由本地化。
- Codex 原生 `--force UUID` 修复非交互提示问题，但本机合成样本试删仍失败；
  原生成功 + 完整恢复保持未验收，数据库替换兜底关闭。Windows 留待后续。

早期检查基线（已由页首最新结果更新）：fmt / Clippy 通过；Rust workspace 159 passed、2 ignored
（本机 read-only probe 与严格 native Trash 测试均另行显式通过）；前端 7 项回归、
类型检查与 Vite build 通过；本机 debug `.app` 打包及原生界面核对通过。
Codex 原生成功 / 完整恢复验收仍保持打开。

## 下一步

优先完成本页“尚未完成”第 1–3 项 Mac 验收，再决定清理能力的发布范围；
不以增加 Provider 数量或视觉完成度替代恢复与真实用户验收。
Phase 8 尚未启动，新资源继续遵循先安全文档、再样本/契约、后启用的流程。

验证环境注意：普通 Rust 测试可能将原生 Trash 拒绝记录为平台失败，只有
显式的严格原生测试通过才算该次恢复验收；前端 pnpm 启动器问题与构建结果
分开记录，不能把 Node 直接入口通过写成 pnpm 命令已恢复。

## 维护约定

- 完成或范围变化时，更新对应 Phase 的状态、关键产出和“当前工作”内容。
- 仅当阶段验收条件已满足时标记为“已完成”；已知阻塞和跨平台差异必须保留。
- 发布面向用户的功能或修复时，同时更新根目录 `CHANGELOG.md`。
