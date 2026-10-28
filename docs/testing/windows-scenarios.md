# 本机 Windows 场景验收

2026-10-28，Windows 10/11（`C:\Users\18712`），Rust 1.9x，Node v24.15.0，pnpm 8.15.5。
这是**本机 Windows 验收记录**，不代表 macOS、其他 Windows 版本或正式发布均已通过。
真实 Provider 数据只做扫描与预览；执行/恢复验证使用临时合成数据。
会话数量为验收时的一次快照，后续会随真实 Agent 活动变化。
不停止真实 Agent、不清空回收站、不复制会话正文或凭证进仓库。

与 [macos-scenarios.md](./macos-scenarios.md) 的关系：两页各自记录本机结论，
**不从一侧推导另一侧通过**。Windows 特有的失败（文件锁、回收站结构、权限、
长路径、保留名、junction / 云占位 reparse point）在本页如实记录。

## 最新验证结论（2026-10-28）

| 检查 | 最近结果 | 范围与限制 |
| --- | --- | --- |
| Rust fmt | 通过 | `cargo fmt --all --check` |
| Rust Clippy | 通过 | warnings denied；本轮修掉了 Windows 专属的 `unused_mut` |
| Rust workspace | **185 项通过、0 失败、2 项默认忽略** | 不将 ignored 计作通过；未对真实会话执行清理 |
| 前端回归 | **19 项通过** | 含身份关联、回执、双语启动、错误状态 |
| TypeScript / Vite | 通过 | `pnpm build:desktop`；`tsc -b` 即类型门禁 |
| **Windows 原生回收站往返** | **严格验收通过** | Recycle Bin 真实 `$I`/`$R` 往返；文件、嵌套目录、空格、中文、引号 |
| 真实只读快照 | **5 个安装、57 个会话** | Claude Code CLI/Desktop、Codex、WorkBuddy、Antigravity；数量随真实活动变化 |
| 真实只读 opt-in 探针 | 显式通过 | `local_readonly_scan_and_preview`（跨平台，已去掉 macOS 断言） |
| Codex 完整清理与恢复 | **未验收，生产清理保持 Blocked** | native delete 与 DB 替换均关闭；仅候选预览 |
| Pi | **不可见（按设计）** | 本机未安装 Pi，无 Windows 样本，`discover()` 保持 `None` |
| 发行包（签名/公证） | 未验证 | 本轮只做了 debug 构建 |

## 本轮修复的 Windows 阻塞缺陷（曾使 CI 为红）

| 缺陷 | 现象 | 处理 |
| --- | --- | --- |
| `atomic_file.rs` 的 `unused_mut` | `clippy -D warnings` 在 Windows 失败 | `mut` 绑定收进 `#[cfg(unix)]` 分支 |
| `MutableSqliteDb::drop` 泄漏临时目录 | Windows `remove_dir_all` 撞上仍存活的连接句柄 | 先换成内存连接再删目录（同 `ReadOnlyDb::drop`） |
| WorkBuddy 契约测试句柄未关 | Windows error 32「另一个程序正在使用此文件」 | 删目录前显式 `drop(db)` |
| 前端回归用目录符号链接 | Windows `symlink` EPERM（需开发者模式） | 改用 `junction`（免提权），非 Windows 仍用 `dir` |

## 本轮修掉的数据正确性缺陷（两端同时受影响）

- **占用口径（§8）**：`UsageAccumulator.allocated_bytes` 由 `None` 起算，
  首次累加又塌回 `None`，于是**所有平台的"分配大小"永远是 null**。
  修前本机 47 个会话类资源中 45 个为 null；修后 **0 个为 null**
  （共享数据库类资源按设计单独计量，不走该累加器，其 `allocated_bytes`
  仍为 null——这是"共享库不摊入会话独占大小"的口径，不是缺陷）。
- **只读 SQLite 读取档位**：`mode=ro` 打开缺少 `-shm` 的 WAL 库会**惰性成功**，
  真正失败要到第一条语句才出现，于是私有副本降级从不触发。
  现在打开后先跑一次真实查询验证可读，并在打开前校验 SQLite 文件头，
  避免把非数据库文件读成空库。
- **独占归属（跨平台同步）**：`is_single_link_file` 在所有非 unix 平台恒返回
  `false`，Windows 上**没有任何会话能被证明独占**，因而永远产不出清理单元。
  现在读取 `BY_HANDLE_FILE_INFORMATION.nNumberOfLinks`（Windows 的 `st_nlink`）。
  修后本机 WorkBuddy 41/42 资源为 exclusive、Codex 2/3。
  CLI golden 基线随之从 `null` 变为既有的 `<ALLOCATED>` 占位符。

## 场景与证据

| 场景 | 当前结果 | 验证入口 / 边界 |
| --- | --- | --- |
| 无安装 / 空目录 | 自动化通过 | CLI empty-home golden；保持只读空结果 |
| 五个 Provider 发现、诊断与扫描 | **本机通过** | Claude Code CLI 1 + Desktop 1、Codex 2、WorkBuddy 41、Antigravity 12，共 57 个会话 |
| Antigravity Windows 接入 | **本机通过** | 状态根、索引与三棵已验证目录与 macOS 同构；投影查询原样可跑；12 条索引、760 个资源 |
| Antigravity 空标题降级 | 自动化通过 | Windows 样本 12 行标题全为空 → 降级为"无标题"，不显示空白标题 |
| Antigravity Windows 盘符 URI | 自动化通过 | `file:///c%3A/...` → `c:\Sample\project`（`Url::to_file_path`） |
| Pi | **不适用** | 本机无安装、无样本；Windows 上按红线保持关闭 |
| Agent 正在运行 | 本机通过 | Codex 报告运行中（`agent_running: true`）；Codex CLI 缺失如实报诊断 |
| Codex 重复 rollout / 归档冲突 | 本机安全降级通过 | 保留诊断与阻断，不猜测哪一份可删除 |
| DB 不存在、损坏或结构不符 | 自动化通过 | Provider 缺库 / malformed / contract 测试；未知生命周期不启用清理 |
| 一个不安全单元与健康单元共存 | 自动化通过 | 不安全行显示 Blocked 及原因，健康行仍在预览中 |
| 路径、符号链接、硬链接 | 自动化通过 | infrastructure / provider safety tests；canonical path 与成员元数据重验 |
| 硬链接独占判定（Windows） | **本机通过** | `nNumberOfLinks` 判定独占；同一物理文件的第二个链接不再被当作独占资源 |
| 只读 WAL 库缺少 `-shm` | 自动化通过 | 打开时验证可读；数据可读、完整性检查通过 |
| 非数据库文件 | 自动化通过 | 文件头校验后 fail-closed；不被私有副本读成空库 |
| **Windows 原生回收站：文件 / 嵌套目录** | **严格验收通过** | Recycle Bin `$I`/`$R` 真实往返；identity 与字节内容不变 |
| **空格、中文、引号路径** | **严格验收通过** | 同一原生测试；只定位自己唯一命名的样本，不遍历其他回收站内容 |
| 文件被占用（锁定） | 自动化通过 | `ERROR_SHARING_VIOLATION` / `ERROR_LOCK_VIOLATION` → `Locked` → `skipped` + 重试提示 |
| 并发执行 / 进程崩溃 / 很老的活跃锁 | 自动化通过 | OS advisory lock；子进程竞争与终止测试；不按文件年龄抢锁 |
| 审计目录不可写 / 崩溃留下半行 | 自动化通过 | 执行前打开 sink、同步 Pending intent；半行与后续事件分隔 |
| 回收空间口径 | 已修复 | 移入回收站仍占空间，不以单元估算冒充实际释放，不自动清空回收站 |
| SQLite 备份与 WAL | 自动化通过 | 只读源 VACUUM INTO；包含尚在 WAL 的提交；不覆盖已有恢复点 |
| 计划过期与二次确认 | 前端门禁测试通过 | TTL 在 Review 与弹窗中实时更新，点击时再次检查 |
| 快速重复点击、刷新旧响应 | 前端测试通过 | CleanupRequests 串行门禁；执行尝试后预览作废 |
| 搜索、Agent / 生命周期 / 时间筛选、排序、分页 | 前端测试通过 | 缺失日期不当作很旧；每页 50 行、空结果可重置 |
| Codex 原生删除成功与完整恢复 | **未通过验收** | 合成样本试删失败；成功路径与完整恢复不能据版本号推定 |
| 桌面应用真机运行与打包 | **未验证** | 本轮未执行 `pnpm tauri dev` / NSIS+MSI 打包 |
| 跨卷 / 外接盘回收站 | **未验证** | 本轮仅覆盖系统盘 |

普通 Rust 测试允许原生回收站在受限会话中返回平台错误；这不算原生恢复验收。
上表"严格验收"使用下述 ignored 测试，任何拒绝均明确失败，不跳过。

## 可重复运行

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 本机元数据与预览：不调用 cleanup_execute，不写真实审计或操作锁。
cargo test -p agenttidy-application tests::live::local_readonly_scan_and_preview -- --ignored --exact --nocapture --test-threads=1

# Windows 原生回收站验收：只新建、移入并恢复自己的临时样本，需要访问系统回收站。
cargo test -p agenttidy-infrastructure windows_acceptance::native_trash_and_manual_restore_preserve_file_and_directory -- --ignored --exact --nocapture --test-threads=1

pnpm test:desktop
pnpm build:desktop
```

前端测试在 `apps/desktop` 下运行 `node --test tests/*.test.mjs`（`pnpm test:desktop`
是同一入口）。本机 pnpm 启动器可用时直接用 pnpm；不可用时用 Node 直接入口，
两者结果一致，但不得把"Node 直接入口通过"写成"pnpm 命令已恢复"。

## 尚需收尾

- 桌面应用在 Windows 上的真机运行（WebView2）、NSIS 与 MSI 打包、
  安装后启动与 `.ico` 核对，以及 WebView2 缺失时的降级提示。
- 跨卷与外接盘的回收站往返；长路径、保留名（`CON`/`PRN` 等）、
  junction 与云占位 reparse point 的真机边界。
- Windows 上的发行包体验：代码签名、安装包签名与升级路径未验证。
- Codex 原生成功路径与完整恢复仍是两端都未验收的项，Windows 不得因
  "没试过"而默认放行。
- 受控 Provider 样本的完整 GUI 点击 → 执行 → 恢复 → 原 Agent 兼容性验收，
  随后开展小范围真实用户 Beta。
- Pi 的 Windows 样本：先安装并采集，再决定路径与格式，然后才放开平台门禁。
