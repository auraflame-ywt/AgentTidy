# Antigravity：macOS 只读接入

调查日期：2026-10-21。本机安装 Antigravity 2.24.0 和 Antigravity IDE 2.5.5；
本轮接入已观察到的 `~/.gemini/antigravity` 状态目录，按数据根报告一次安装，
不把两个 app bundle 重复计入。其他版本、其他目录与 Windows 尚未验证。
对应 `Start.md` §10、§18、§22。

## 已验证结构

- `conversation_summaries.db`：真实 `conversation_summaries` 表的 50 条索引。
  仅投影 conversation_id、title、last_modified_time、workspace_uris、status。
  不读取 preview、raw_summary 或含正文的 FTS 表。
- `conversations/<UUID>.pb`：二进制正文，不解码；只测量文件大小。
- `brain/<UUID>/...`：按相同索引 id 关联的产物文件，只枚举和测量，不读取正文。
- 数据库及 WAL/SHM 是 Shared 资源，计入安装占用，不摊入会话独占大小。
  使用基础设施 ReadOnlyDb，保留 WAL 可见性及受限环境的私有副本降级；
  禁止用 immutable 读取活动数据库而遗漏 WAL。链接数据库/sidecar 拒绝读取。

索引 id 必须符合 UUID 形状，不能控制根目录；读取范围固定为上述已验证目录。
所有树遍历不跟随链接，硬链接按物理 identity 去重。

## 映射与降级

- 索引标题直接展示；时间按带时区日期解析为毫秒，负纪元/无效日期保持未知。
- workspace_uris 是 JSON 字符串数组；单个 file URI 解码为项目路径，
  多工作区不强行归到第一个项目，详情保留完整关联列表。不遍历这些项目目录。
- 本机观察到 `CASCADE_RUN_STATUS_IDLE`，保留为 index_status；它不能证明
  会话可清理，所以生命周期和所有会话文件的归属均为 Unknown。
- 索引缺失/不可读/投影字段变化会显示诊断，不从不透明 protobuf 猜造会话。
  缺失对应正文、未索引文件和枚举失败会报告，部分占用标为 Estimated。
  include_unknown 可纳入两棵树里的未归属文件，仍为 Unknown resource。
- 只测量已验证索引、会话和 brain 文件；缓存、knowledge、其他共享存储、
  Electron 数据目录及另一个 `~/.antigravity` 扩展根不在本轮占用范围。

Sessions / Projects 为 ReadOnly，索引问题降级；Archive / RestoreArchive /
Cleanup / Cache / Logs 不支持。不产生 cleanup unit，不报告 reclaimable bytes。
protobuf、索引、共享状态与产物的完整恢复尚未验证，因此没有安全清理文档，也不启用清理。

脱敏实样：`fixtures/antigravity/index/{schema.sql,row.json}` 来自真实 DDL 与
元数据投影，替换标题/id/工作区 URI，排除所有正文和凭据。测试中的占位字节仅用于
验证文件枚举，不作为 protobuf 格式样本。覆盖 schema 漂移、WAL、文件缺失、
未知文件、UUID 边界、链接/硬链接、多工作区与 CLI 接入。

本机只读验收：50 条索引、1,206 个已纳入资源；其中 4 条索引无可读对应正文，
另外检测到未索引文件。均保留诊断，没有修改或清理真实会话。
