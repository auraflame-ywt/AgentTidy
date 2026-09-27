// Chinese (Simplified) catalog. Typed as `Record<TranslationKey, string>`
// so `tsc -b` fails on any missing OR extra key — the catalog cannot drift
// from `en.ts`. `.one` / `.other` pairs carry the same Chinese text (no
// plural rules). Brand names (AgentTidy, Codex, Claude Code, WorkBuddy),
// IEC units, and functional tokens (CONFIRM) intentionally stay as-is.

import type { TranslationKey } from "./en";

export const zh: Record<TranslationKey, string> = {
  // --- App chrome ---
  "tab.overview": "概览",
  "tab.sessions": "会话",
  "tab.tidy": "整理",
  "tab.diagnostics": "诊断",
  "header.tagline": "本地优先的 Agent 存储检查器 · 回收站优先 · 失败即关闭",
  "common.refresh": "刷新",
  "common.scanning": "扫描中…",
  "common.failure": "失败：",

  // --- Empty state ---
  "empty.title": "未检测到受支持的 Agent 安装",
  "empty.body":
    "AgentTidy 会在本机查找 Codex、Claude Code 和 WorkBuddy。安装其中之一，或以更高权限运行，即可在此看到检测结果。",

  // --- Overview ---
  "overview.noInstallations": "尚未检测到任何安装，试试刷新。",
  "overview.sessions": "会话",
  "overview.resources": "资源",
  "overview.totalSize": "总大小",
  "overview.workspaceFootprint": "工作区占用",
  "overview.workspaceValue.one": "{count} 个资源 · {bytes}",
  "overview.workspaceValue.other": "{count} 个资源 · {bytes}",
  "overview.diagnostics.one": "{count} 条诊断 →",
  "overview.diagnostics.other": "{count} 条诊断 →",

  // --- Installation status badges ---
  "status.available": "可用",
  "status.permissionRequired": "需要权限",
  "status.unsupported": "不支持",

  // --- Capability status badges ---
  "capability.unsupported": "不支持",
  "capability.permissionRequired": "需要权限",
  "capability.degraded": "降级",
  "capability.readOnly": "只读",
  "capability.supported": "支持",

  // --- Capability topic labels ---
  "topic.sessions": "会话",
  "topic.projects": "项目",
  "topic.archive": "归档",
  "topic.cache": "缓存",
  "topic.logs": "日志",

  // --- Size confidence ---
  "confidence.exact": "精确",
  "confidence.estimated": "估算",
  "confidence.unknown": "未知",

  // --- Session lifecycle ---
  "lifecycle.active": "活跃",
  "lifecycle.inactive": "已结束",
  "lifecycle.unknown": "未知",
  "lifecycle.archived": "已归档",

  // --- Provider / platform display names (brands stay Latin) ---
  "provider.claudeCode": "Claude Code",
  "provider.codex": "Codex",
  "provider.workbuddy": "WorkBuddy",
  "platform.macos": "macOS",
  "platform.windows": "Windows",

  // --- Sessions view ---
  "sessions.empty.title": "未识别到任何会话",
  "sessions.empty.body":
    "当前检测到的安装没有产出任何会话。通常意味着状态目录为空，或本机尚未使用过受支持的 Agent。",
  "sessions.col.installation": "安装",
  "sessions.col.session": "会话",
  "sessions.col.project": "项目工作目录",
  "sessions.col.lifecycle": "生命周期",
  "sessions.col.size": "大小",
  "sessions.col.updated": "最后更新",

  // --- Diagnostics view ---
  "diag.empty": "检测到安装后，诊断信息会显示在这里。",
  "diag.clean": "所有已检测安装均无检查警告，检测与检查流程正常完成。",
  "diag.readableRoots": "可读根目录",
  "diag.readable": "可读",
  "diag.notReadable": "不可读",
  "diag.schemaVersions": "Schema 版本",
  "diag.journalModes": "日志模式",
  "diag.dataRoots": "声明的数据根目录",
  "diag.problems.one": "{count} 条问题",
  "diag.problems.other": "{count} 条问题",
  "severity.warning": "警告",
  "severity.error": "错误",

  // Stable ScanProblem codes — Chinese messages; the dynamic suffix
  // (path / id) is appended verbatim by translateDiagnostic().
  "problem.thread-index-missing": "缺少 state_5.sqlite；仅 JSONL 扫描将降级",
  "problem.thread-index-unavailable": "线程索引不可用；回退为仅 JSONL 扫描",
  "problem.projects-missing": "projects 目录缺失",
  "problem.projects-unreadable": "projects 目录不可读",
  "problem.desktop-sessions-missing": "Claude Desktop 没有 local-agent-mode 会话",
  "problem.workbuddy-db-journal": "无法读取 workbuddy.db 的 journal 模式",
  "problem.workbuddy-db-unavailable": "workbuddy.db 不可用",
  "problem.unreadable": "路径不可读：",
  "problem.unrecognized-transcript": "无法识别的 transcript（已跳过）：",
  "problem.unrecognized-rollout": "无法识别的 rollout（已跳过）：",
  "problem.duplicate-session": "重复的 session id（未合并）：",
  "problem.thread-index-missing-session": "rollout 不在线程索引中：",
  "problem.archive-state-mismatch": "归档状态与线程索引不一致：",
  "problem.project-unreadable": "无法读取项目目录：",
  "problem.workspace-unreadable": "工作区路径不可读：",

  // --- Review (Tidy) view ---
  "risk.lowRisk": "建议清理",
  "risk.reviewRequired": "谨慎清理",
  "risk.blocked": "禁止清理",
  "review.loading": "正在加载清理计划…",
  "review.found.one": "发现 {bytes}，共 1 个清理单元",
  "review.found.other": "发现 {bytes}，共 {count} 个清理单元",
  "review.selected": "已选",
  "review.planId": "计划 {id}",
  "review.expires": "{when} 过期",
  "review.back": "返回",
  "review.cleanNow": "立即清理",
  "review.reviewSelected": "审查所选",
  "review.phase7.before":
    "最近一次扫描没有发现清理单元。Provider 清理按资源类型逐步启用（Start.md §19）——Claude Code 会话清理 Beta 已开放；其他资源类型通过各自安全门槛后陆续启用。每种资源类型在启用清理前都会获得自己的 ",
  "review.phase7.after": " 文档。",
  "review.bucket.recommended": "{count} 项建议清理",
  "review.bucket.caution": "{count} 项谨慎清理",
  "review.bucket.offLimits": "{count} 项禁止清理",
  "review.executionResults": "执行结果",
  "outcome.pending": "待执行",
  "outcome.removed": "已移除",
  "outcome.skipped": "已跳过",
  "outcome.failed": "失败",
  "review.stale": "已变化 — 重新扫描",
  "review.revalidatePassed": "重新验证通过——所选单元自计划生成以来均未发生变化。",
  "review.revalidateChanged.one":
    "1 个单元自计划生成以来发生变化，已从选择中移除（§12.2）。请返回并刷新以重新生成计划。",
  "review.revalidateChanged.other":
    "{count} 个单元自计划生成以来发生变化，已从选择中移除（§12.2）。请返回并刷新以重新生成计划。",
  "review.ariaSelect": "选择 {id}",
  "review.expired":
    "计划已过期——请重新扫描（返回 → 刷新）后再审查或清理（§12.2）。",
  "review.item.action.trash": "移入回收站",
  "review.item.action.providerOperation": "Agent 专属操作",
  "review.item.recoverable.trash": "可从系统回收站恢复",
  "review.item.recoverable.providerOperation": "不可通过回收站恢复",
  "review.item.members.one": "单元包含 1 个路径",
  "review.item.members.other": "单元包含 {count} 个路径",

  // --- ReviewConfirm modal ---
  "confirm.title": "确认清理",
  "confirm.body.one":
    "你即将把 1 个单元（{bytes}）移入系统回收站。恢复由 macOS / Windows 处理——AgentTidy 不执行任何永久删除。",
  "confirm.body.other":
    "你即将把 {count} 个单元（{bytes}）移入系统回收站。恢复由 macOS / Windows 处理——AgentTidy 不执行任何永久删除。",
  "confirm.plan": "计划",
  "confirm.fingerprint": "指纹",
  "confirm.reclaimable": "可回收",
  "confirm.typeBefore": "输入 ",
  "confirm.typeAfter": " 以启用执行按钮。",
  "confirm.cancel": "取消",
  "confirm.moveToTrash": "移入回收站",
};