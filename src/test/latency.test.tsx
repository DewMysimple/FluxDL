import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import {
  TaskDiagnosticStatus,
  waitingMilliseconds,
  observedStatus,
} from "../features/diagnostics/TaskDiagnosticStatus";
import {
  diagnosticsApi,
  taskDiagnosticSummary,
} from "../features/diagnostics/diagnostics-api";
import {
  phaseLabel,
  type DiagnosticEvent,
  type Health,
} from "../features/diagnostics/types";
vi.mock("../lib/api", () => ({ isDesktop: true, call: vi.fn() }));
const session = "11111111-1111-4111-8111-111111111111";
const trace = "22222222-2222-4222-8222-222222222222";
const linked = "33333333-3333-4333-8333-333333333333";
function event(overrides: Partial<DiagnosticEvent> = {}): DiagnosticEvent {
  return {
    schema_version: 1,
    event_id: crypto.randomUUID(),
    session_id: session,
    trace_id: trace,
    span_id: trace,
    parent_span_id: null,
    task_id: trace,
    timestamp_utc: "2099-01-01T00:00:00Z",
    elapsed_since_session_start_ms: 100,
    level: "info",
    source: "rust",
    event: "stage_started",
    phase: "download_first_progress",
    status: "started",
    duration_ms: null,
    fields: { related_trace_id: null, attempt_id: trace, error_code: null },
    ...overrides,
  };
}
const health: Health = {
  mode: "development",
  sessionId: session,
  directory: null,
  writeRestricted: false,
  dropped: 0,
  criticalDropped: 0,
  generation: 0,
  level: "info",
  elapsedSinceSessionStartMs: 40000,
};
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
describe("真实等待与安全摘要", () => {
  it("只在同会话Rust时钟内计算已等待时间，忽略UTC与前端时间戳", () => {
    expect(waitingMilliseconds([event()], health)).toBe(39900);
    expect(
      waitingMilliseconds([event({ session_id: trace })], health),
    ).toBeNull();
    expect(
      waitingMilliseconds([event()], {
        ...health,
        elapsedSinceSessionStartMs: undefined,
      }),
    ).toBeNull();
    expect(
      waitingMilliseconds(
        [event({ elapsed_since_session_start_ms: 50000 })],
        health,
      ),
    ).toBeNull();
    expect(
      waitingMilliseconds(
        [event({ status: "ok", duration_ms: 2 }), event()],
        health,
      ),
    ).toBeNull();
  });
  it("明确表达已输出及下载前标记，不推断网络连接或首字节", () => {
    expect(
      observedStatus([event({ phase: "download_first_output", status: "ok" })]),
    ).toContain("引擎已有输出");
    expect(
      observedStatus([event({ phase: "download_before_dl", status: "ok" })]),
    ).toContain("已观察下载前标记");
    expect(phaseLabel("future_stage")).toContain("未知阶段");
    expect(observedStatus([])).toBeNull();
  });
  it("真正超过阈值才提示长等待，业务终态立即清除准备提示", async () => {
    const taskId = crypto.randomUUID();
    vi.spyOn(diagnosticsApi, "query").mockResolvedValue({
      events: [event()],
      total: 1,
      nextCursor: null,
      unreadableLines: 0,
      truncated: false,
    });
    vi.spyOn(diagnosticsApi, "health").mockResolvedValue(health);
    const view = render(
      <TaskDiagnosticStatus taskId={taskId} active status="resolving" />,
    );
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("已等待 39 秒"),
    );
    expect(screen.getByRole("status")).toHaveTextContent("此步骤等待较久");
    expect(screen.getByRole("status")).not.toHaveTextContent(
      /%|ETA|连接已完成/,
    );
    view.rerender(
      <TaskDiagnosticStatus
        taskId={taskId}
        active={false}
        status="completed"
      />,
    );
    expect(screen.queryByRole("status")).toBeNull();
  });
  it("诊断不可读时安静降级，不伪造阶段或计时", async () => {
    vi.spyOn(diagnosticsApi, "query").mockRejectedValue(
      new Error("fixture unavailable"),
    );
    vi.spyOn(diagnosticsApi, "health").mockResolvedValue(health);
    render(
      <TaskDiagnosticStatus
        taskId={crypto.randomUUID()}
        active
        status="resolving"
      />,
    );
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("阶段记录暂不可用"),
    );
    expect(screen.getByRole("status")).not.toHaveTextContent("已等待");
  });
  it("复制当前尝试及关联解析，缺失耗时不补零，旧任务明确降级", async () => {
    const row = event({
      phase: "download_before_dl",
      status: "unobserved",
      fields: {
        related_trace_id: linked,
        attempt_id: trace,
        error_code: null,
        environment_id: session,
      },
    });
    vi.spyOn(diagnosticsApi, "query").mockImplementation(async (filter) => ({
      events:
        filter?.traceId === linked
          ? [
              event({
                trace_id: linked,
                fields: {
                  related_trace_id: null,
                  attempt_id: null,
                  error_code: null,
                },
                phase: "parse_total",
                status: "ok",
                duration_ms: 12,
              }),
            ]
          : [row],
      total: 1,
      nextCursor: null,
      unreadableLines: 0,
      truncated: false,
    }));
    vi.spyOn(diagnosticsApi, "overview").mockResolvedValue({
      slowTraces: [],
      sessionId: session,
      parses: 1,
      downloads: 1,
      errors: 0,
      total: 1,
      statistics: [
        {
          phase: "download_before_dl",
          label: "fixture",
          samples: 1,
          p50Ms: 1,
          p95Ms: 1,
          maxMs: 1,
          slowestTraceId: trace,
          sessionId: session,
          environmentId: session,
          codeRevision: "a".repeat(40),
          engineVersions: { "yt-dlp": "2026.07.04" },
        },
      ],
      recentErrors: [],
      unreadableLines: 0,
      truncated: false,
      fromUtc: null,
      toUtc: null,
    });
    const text = await taskDiagnosticSummary(trace);
    expect(text).toContain(`当前尝试：${trace}`);
    expect(text).toContain(`Trace：${linked}`);
    expect(text).toContain("未观察到完整耗时");
    expect(text).toContain("12.000 ms");
    expect(text).toContain("首次有效进度不是网络首字节");
    expect(text).not.toContain("0.000 ms");
    expect(await taskDiagnosticSummary("legacy-id")).toContain("旧任务");
  });
});
