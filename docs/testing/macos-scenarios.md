# 本机 macOS 场景验收

2026-10-20，macOS 27.0.1 / arm64，Rust 1.94.0。
这是本机验收记录，不代表其他 Mac、Windows 或正式发布均已通过。
真实 Provider 数据只做扫描与预览；执行/恢复验证使用临时合成数据。
会话数量为验收时的一次快照，后续会随真实 Agent 活动变化。
不停止真实 Agent、不清空回收站、不复制会话正文或凭证进仓库。

## 最新验证结论（本轮结束）

| 检查 | 最近结果 | 范围与限制 |
| --- | --- | --- |
| Rust fmt / Clippy | 通过 | Clippy warnings denied；见本轮代码修复后的检查记录 |
| Rust workspace | 165 项通过、2 项默认忽略 | 不将 ignored 计作通过；未对真实会话执行清理 |
| 前端回归 | 17 项通过 | 包含身份关联、结果回执、双语启动及错误状态 |
| TypeScript / Vite | 通过 | 使用已安装 Node 的直接入口，pnpm 启动器问题未宣称解决 |
| Mac debug `.app` | 打包及原生界面核对通过 | 新图标与包内 ICNS 一致；不是签名/公证发行包 |
| 真实只读快照 | 3 个安装、354 个会话 | Claude Code 26、Codex 19、WorkBuddy 309；数量和运行状态会变化 |
| 本机只读 opt-in / 严格 Trash 恢复 | 较早阶段分别显式通过 | 使用临时样本；后续品牌升级未重跑严格 Trash，不计入 165 项 |
| Codex 完整清理与恢复 | 未验收，生产清理 Blocked | native delete 与 DB 替换均关闭；仅支持候选预览 |
| Windows / 跨卷恢复 / 发行包体验 | 未在本轮验证 | 不从 Mac 结果推导通过 |

汇总和后续顺序见 [项目进度](../CHANGELOG.md)。
下文保留分阶段证据；153/159/162 等早期测试数量不是最新基线。

## 首轮场景与证据（历史快照）

| 场景 | 当前结果 | 验证入口 / 边界 |
| --- | --- | --- |
| 无安装 / 空目录 | 自动化通过 | CLI empty-home golden；保持只读空结果 |
| 三 Provider 发现、诊断与扫描 | 本机通过 | Claude Code 26、Codex 18、WorkBuddy 309，共 353 个会话；只读元数据 |
| Agent 正在运行 | 本机通过 | 三个 Provider 均识别为运行中；原生 `.app` 的 337 个清理单元全部 Blocked，选择与确认不可用 |
| WorkBuddy 的 Electron 主进程 | 已修复并本机核对 | 5.8.3 的 CFBundleExecutable 为 Electron；通过 WorkBuddy.app/Contents 路径识别，其他 Electron 应用不归属 WorkBuddy |
| Codex 重复 rollout / 归档冲突 | 本机安全降级通过 | 保留诊断与阻断，不猜测哪一份可删除；不自动合并或修复真实数据 |
| DB 不存在、损坏或结构不符 | 自动化通过 | Provider 缺库 / malformed / contract 测试；未知生命周期不启用清理 |
| 一个不安全单元与健康单元共存 | 自动化通过 | 不安全行显示 Blocked 及原因，健康行仍在预览中；Blocked 大小不计可处理总额 |
| 默认工作区 / 用户项目 | 自动化通过 | report-only；Workspace 安全事实和自动化引用门禁，不产生清理执行路径 |
| 路径、符号链接、硬链接 | 自动化通过 | infrastructure / provider safety tests；canonical path 与成员元数据重验，不跟随越界链接 |
| 附件增加、替换、消失 / 会话重激活 / Schema 变化 | 自动化通过 | Application live regressions；执行前逐项重建当前单元并重新评估策略 |
| 快速重复点击、刷新旧响应 | 前端测试通过 | CleanupRequests 串行门禁；取消后的响应不能恢复旧状态；执行尝试后预览作废 |
| 计划过期与二次确认 | 前端门禁测试通过 | TTL 在 Review 与弹窗中实时更新，点击时再次检查；取消不执行；混合 Provider 操作不误标为 Move to Trash |
| 搜索、Agent / 生命周期 / 时间筛选、排序、分页 | 前端测试 + 本机界面通过 | 搜索标题 / id / 项目路径；缺失日期不当作很旧；每页 50 行、空结果可重置；实际 UI 核对搜索空结果与 Codex 18 条筛选 |
| 原生 Mac 回收站：文件 / 嵌套目录 | **严格验收通过** | NSFileManager；临时样本移入并手动移回，identity 与字节内容不变 |
| 空格、中文、引号路径 | **严格验收通过** | 同一原生测试；只访问唯一命名的测试 Trash 路径，不遍历其他回收站内容 |
| Mac sandbox / 权限拒绝 | 已验证明确失败 | 受限执行中原生 API 报权限拒绝；不执行永久删除兜底。严格原生验收在获准的本机权限下通过 |
| 并发执行 / 进程崩溃 / 很老的活跃锁 | 自动化通过 | OS advisory lock；真实子进程竞争和终止测试；不按文件年龄抢锁，不 unlink 活跃锁路径 |
| 旧版本 lock stamp | 自动化通过 | 未知旧锁 fail-closed，不自动删除；需先关闭旧客户端并人工核对 |
| 审计目录不可写 / 崩溃留下半行 | 自动化通过 | 执行前打开 sink、同步 Pending intent；不能写入则不执行；半行与后续事件分隔；结果写失败停止剩余批次 |
| 回收空间口径 | 已修复 | 移入 Trash 仍占空间，actual reclaimed_bytes=0；不以单元估算冒充实际释放，不自动清空 Trash |
| SQLite 备份与 WAL | 自动化通过 | 只读源 VACUUM INTO；包含尚在 WAL 的提交；独立快照、quick_check、fsync，不覆盖已有恢复点 |
| Mac 应用打包与运行 | 本机通过 | Tauri debug `.app`，打开实际 WebView 并核对本机数据；不是签名/公证发行包 |
| Codex 原生删除成功与完整恢复 | **未通过验收** | 本机 0.152.0 对隔离合成会话返回 failed to delete session；合成数据未删。成功路径、完整文件/跨库恢复不能据 help / 版本号推定 |

普通 Rust 测试允许原生 Trash 在受限会话中返回平台错误；这不算原生恢复
验收。上表“严格验收”使用下述 ignored 测试，任何拒绝均明确失败，不跳过。

首轮自动化基线（最新结果见页首）：Rust fmt / Clippy 均通过，workspace **159 项通过、2 项
opt-in 测试默认忽略**；这 2 项本机只读 / 严格原生验收另行显式运行均通过。
前端 **7 项场景回归**、TypeScript 和 Vite build 均通过。调试 `.app` 打包、
启动和原生界面核对通过。Codex 合成样本试删的失败独立记录，未计作成功。

## 检查后修复复验（2026-10-20）

- Codex 完整恢复门槛：结构有效的归档样本也被阻断；直接 Provider 执行
  返回 Failed，原始 rollout 和 thread row 保持存在，生产代码不再包含删除入口。
- CLI 预检：缺失 / 非零退出 / 不支持版本均覆盖；CLI golden 隔离 PATH，
  明确验证诊断，前端同时解释恢复与 CLI 两类门槛。
- 同库批次：执行入口在锁、审计和修改前整批拒绝；未选中的行不干扰单选；
  canonical 路径别名与硬链接不能绕过，不同数据库仍独立处理。
- 格式检查、Clippy（warnings denied）、全工作区 162 项 Rust 测试通过；
  两项 ignored 原生/本机测试不计入该常规数量。前端 TypeScript/Vite 构建与
  8 项回归通过（仍使用上文说明的本地 Node 直接入口）。
- 修复后另行显式运行本机只读扫描/预览测试通过：26 个 Claude Code、
  19 个 Codex、309 个 WorkBuddy 会话；Codex 两个候选均为 Blocked。
  未执行真实会话清理，未宣称完整恢复或同库批量删除已验收。

## 页面与使用路径优化复验（2026-10-20）

- 原生 debug `.app`：总览识别 3 个安装、354 个会话；占用排行链接以最大
  优先进入列表，详情侧栏展示关联路径、大小、生命周期和处理条件；Escape
  可关闭，背景不可交互。真实会话未执行清理。
- 整理页：首次进入才生成预览；会话/项目名称优先，缺失标题使用已知项目
  与短 ID，不读取正文猜标题。后端风险/阻断及二次确认继续独立生效。
- 结果：仅统计提交的单元；明确部分失败、原始恢复位置、手动移回步骤和
  回收站仍占空间。本轮使用组件回归测试验证失败/未选中结果组合，不以
  真实会话删除验证该页面，也不宣称有跨重启的执行历史或自动恢复。
- 数据关联：必须由同安装的资源引用和预览成员证据唯一关联，不能从 unit ID
  猜会话，不能仅凭共享数据库关联多个会话；歧义时保留技术标签。
- 本机界面发现并核对 Codex 秒/毫秒单位错误，已修正 Provider 边界转换；
  CLI 启动器增加 3 秒超时，超时仍保留只读浏览并显示诊断。
- Node 组件/纯函数回归覆盖中英文、HTML 转义、关联歧义、诊断合并、
  未提交项排除、部分失败及响应缺失时的恢复提示。本轮 165 项 Rust、14 项
  前端回归，以及格式、Clippy、TypeScript/Vite 构建通过。构建入口仍按下文
  环境说明运行。

## 品牌与启动页复验（2026-10-20）

- 新图标：保留透明圆角边缘，检查 512px Web、128px 桌面预览与 32px 导出；
  Tauri CLI 从已提交源图生成 ICNS/ICO/桌面 PNG 和 favicon，移动端未变更。
- 原生窗口：核对深石墨/银白/翡翠绿的启动排版，正常工作台保持浅色；
  “在后台继续”可进入工作台；左上品牌图标可重新打开完整品牌页。
- 启动不增加最短等待时间或伪造进度。扫描完成后以 420ms 过渡退出，
  减弱动态效果时直接退出；失败保留重试/诊断。17 项前端回归及构建通过。
- 本轮只更换品牌资源与启动展示，不触发清理或改变后端清理资格。

## 可重复运行

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 本机元数据与预览：不调用 cleanup_execute，不写真实审计或操作锁。
cargo test -p agenttidy-application tests::live::local_macos_readonly_scan_and_preview -- --ignored --exact --nocapture --test-threads=1

# 原生 Trash 验收：只新建、移入并恢复自己的临时样本，需要访问系统 Trash。
cargo test -p agenttidy-infrastructure macos_acceptance::native_trash_and_manual_restore_preserve_file_and_directory -- --ignored --exact --nocapture --test-threads=1

pnpm test:desktop
pnpm build:desktop
```

当前本机会话的 pnpm 启动器在获取指定版本时出现 registry verification / fetch
失败；实际前端测试用 `node --test tests/*.test.mjs`（在 apps/desktop）运行，
构建直接运行同目录的 `node_modules/.bin/tsc -b` 与 `node_modules/.bin/vite build`。
未关闭签名验证或修改 lockfile 来绕过该环境问题。

调试 `.app` 可在前端构建后通过本地 Tauri CLI 构建：

```bash
# apps/desktop 下；跳过已手工完成的 beforeBuildCommand，其余打包步骤照常执行。
./node_modules/.bin/tauri build --debug --bundles app --config '{"build":{"beforeBuildCommand":""}}'
```

## 尚需收尾

- Codex 原生成功路径与完整恢复必须先取得可重复的本机端到端证据。
  生产清理现已保持 Blocked，直接调用 Provider 也无法执行 native delete；
  CLI 版本检查前移至预览和诊断，不能单凭版本号开启清理。
  同库多选在任何修改前整批拒绝，不尝试重置共享 DB/WAL 指纹。
- 回收站恢复由用户手动移回原位置；NSFileManager 不保证 Finder 的“放回原处”
  按钮。跨卷 / 外接盘、系统 TCC 拒绝及签名公证发行体验尚未在本机严格验证。
- 未关闭用户正在运行的 Agent，因此未对真实会话执行清理；确认弹窗键盘交互
  已实现，完整原生点击到执行闭环仍需使用受控 Provider 样本验收。
- Windows 验收后续单独推进，不由本机结果推断。
