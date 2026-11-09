import { call } from "../../lib/api";
import type {
  DiagnosticFilter,
  DiagnosticPage,
  Overview,
  Health,
} from "./types";
import { phaseLabel, statusLabels } from "./types";
export const diagnosticsApi = {
  health: () => call<Health>("get_diagnostics_health"),
  query: (filter: DiagnosticFilter = {}) =>
    call<DiagnosticPage>("query_diagnostic_events", { filter }),
  overview: (filter: DiagnosticFilter = {}) =>
    call<Overview>("get_diagnostics_overview", { filter }),
  refresh: () => call<void>("refresh_diagnostic_report"),
  open: () => call<void>("open_diagnostics_directory"),
  export: (filter: DiagnosticFilter = {}) =>
    call<string | null>("export_diagnostics_bundle", { filter }),
  reveal: () => call<void>("reveal_diagnostics_bundle"),
};

export async function taskDiagnosticSummary(taskId: string): Promise<string> {
  if (!/^[0-9a-f-]{36}$/.test(taskId))
    return "旧任务无可关联的UUID诊断记录；阶段、版本与耗时均为unknown，不补造。";
  const [page, overview] = await Promise.all([
    diagnosticsApi.query({ taskId, limit: 200 }),
    diagnosticsApi.overview({ taskId }),
  ]);
  const latest =
    page.events.find(
      (e) =>
        e.source === "rust" &&
        e.fields.attempt_id &&
        e.session_id === overview.sessionId,
    ) ?? page.events.find((e) => e.source === "rust" && e.fields.attempt_id);
  if (!latest)
    return `FluxDL任务 ${taskId}：无有效阶段记录；可能超出保留窗口，不能补造耗时。`;
  const sections = [
    `FluxDL安全任务诊断摘要\n匿名任务：${taskId}\n当前尝试：${latest.fields.attempt_id}\n最近记录：${latest.timestamp_utc}`,
  ];
  let trace: string | null = latest.trace_id;
  const seen = new Set<string>();
  for (let depth = 0; trace && depth < 4 && !seen.has(trace); depth++) {
    seen.add(trace);
    const result = await diagnosticsApi.query({ traceId: trace, limit: 200 });
    const spans = [
      ...new Map(
        [...result.events].reverse().map((e) => [e.span_id, e]),
      ).values(),
    ];
    const context = overview.statistics.find(
      (s) =>
        s.sessionId === spans[0]?.session_id &&
        s.environmentId === spans[0]?.fields.environment_id,
    );
    sections.push(
      `Trace：${trace}\n代码revision：${context?.codeRevision ?? "unknown"}\n引擎版本：${JSON.stringify(context?.engineVersions ?? {})}`,
    );
    for (const e of spans)
      sections.push(
        `${phaseLabel(e.phase)} / ${e.status === "started" && e.session_id !== overview.sessionId ? "旧会话未闭合，当前状态未确认" : (statusLabels[e.status] ?? "未知状态")} / ${e.duration_ms == null ? "未观察到完整耗时" : `${e.duration_ms.toFixed(3)} ms`} / ${e.source}`,
      );
    if (result.truncated || result.nextCursor || result.unreadableLines)
      sections.push("记录受保留/分页/损坏窗口限制，不能视为完整Trace。");
    trace =
      result.events.find((e) => e.fields.related_trace_id)?.fields
        .related_trace_id ?? null;
  }
  sections.push(
    "阶段存在嵌套不能相加；首次有效进度不是网络首字节；DNS/TLS及引擎内部等待仍不可细分。前端与Rust单调时钟独立。未包含标题、URL、路径、凭据或原始引擎输出。",
  );
  return sections.join("\n");
}
