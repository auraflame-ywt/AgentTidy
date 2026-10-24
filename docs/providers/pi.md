# Pi：macOS 只读接入

调查日期：2026-10-21。依据本机安装的 `@earendil-works/pi-coding-agent`
0.87.1、`dist/config.js`、`dist/core/session-manager.{js,d.ts}` 与 3 个真实会话。
对应 `Start.md` §10、§18、§22；不是对未来版本或 Windows 的兼容承诺。

## 已验证结构与映射

- 默认 agent root：`~/.pi/agent`，会话位于 `sessions/<project-dir>/*.jsonl`。
- 支持进程环境中的 `PI_CODING_AGENT_DIR`、`PI_CODING_AGENT_SESSION_DIR`，以及
  `~/` 展开；后者作为独立会话目录，不额外拼接 `sessions`。
  AgentTidy 无法获知另一个终端中临时设置的变量，也不读取 Pi 的私有设置文件；
  `--session-dir` 和设置中的额外会话目录尚不自动发现。
- 第一行真实 header 为 `type=session, version=3, id, timestamp, cwd`。
  上游类型还声明 `parentSession`；如果存在，只保留为元数据。
- 解析第一行，上限 64 KiB；不读取后续消息、工具结果或分支正文。
  空行/损坏行前导、旧版迁移和新版本均不猜测兼容，报告诊断。
- id → SessionId，cwd → ProjectRef，ISO timestamp → created_at（毫秒），
  文件 mtime → updated_at（明确不是最后一条消息时间）。不生成会话标题。
- JSONL 文件作为 Session resource；测量逻辑/分配大小，避免重复计算硬链接。
  占用仅覆盖已识别会话，不代表整个 agent root。

## 边界

Sessions / Projects 为 ReadOnly；错误时 Degraded / PermissionRequired。
生命周期、独占归属保持 Unknown，不报告可回收大小，不产生 cleanup unit。
Archive / RestoreArchive / Cleanup / Cache / Logs 不支持。
不读取 auth.json、settings.json、models.json 或会话正文，不运行 Pi CLI。
符号链接根和会话目录不扫描，树内符号链接不跟随。

脱敏实样：`fixtures/pi/sessions/header.jsonl`（id/cwd 已替换、正文未保留）。
覆盖：真实 header 映射、正文不解析、格式变更/超限、重复 id、独立会话根、
链接边界、只读能力和 CLI 默认/环境目录发现。
