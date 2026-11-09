import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { DiagnosticsPage } from "../features/diagnostics/DiagnosticsPage";
import { diagnosticsApi } from "../features/diagnostics/diagnostics-api";
import {
  observedStatus,
  TaskDiagnosticStatus,
} from "../features/diagnostics/TaskDiagnosticStatus";
import type {
  DiagnosticEvent,
  Health,
  Overview,
} from "../features/diagnostics/types";
vi.mock("../lib/api", () => ({ isDesktop: true }));
vi.mock("../features/diagnostics/diagnostics-api", () => ({
  diagnosticsApi: {
    health: vi.fn(),
    query: vi.fn(),
    overview: vi.fn(),
    open: vi.fn(),
    refresh: vi.fn(),
    export: vi.fn(),
    reveal: vi.fn(),
  },
}));
const trace = "11111111-1111-4111-8111-111111111111";
const task = "22222222-2222-4222-8222-222222222222";
const event: DiagnosticEvent = {
  schema_version: 1,
  event_id: trace,
  session_id: trace,
  trace_id: trace,
  span_id: trace,
  parent_span_id: null,
  task_id: task,
  timestamp_utc: "2026-11-09T08:00:00Z",
  elapsed_since_session_start_ms: 100,
  level: "info",
  source: "rust",
  event: "stage_finished",
  phase: "download_first_progress",
  status: "unobserved",
  duration_ms: null,
  fields: { related_trace_id: null, attempt_id: task, error_code: null },
};
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(diagnosticsApi.health).mockResolvedValue({
    mode: "development",
    sessionId: trace,
    directory: "fixture/logs",
    writeRestricted: false,
    dropped: 0,
    criticalDropped: 0,
    generation: 1,
    level: "info",
  } satisfies Health);
  vi.mocked(diagnosticsApi.overview).mockResolvedValue({
    sessionId: trace,
    parses: 0,
    downloads: 0,
    errors: 0,
    total: 0,
    statistics: [],
    slowTraces: [],
    recentErrors: [],
    unreadableLines: 0,
    truncated: false,
    fromUtc: null,
    toUtc: null,
  } satisfies Overview);
  vi.mocked(diagnosticsApi.query).mockResolvedValue({
    events: [],
    total: 0,
    nextCursor: null,
    unreadableLines: 0,
    truncated: false,
  });
});
afterEach(cleanup);
it("ranks slow traces from the bounded analysis window independently of the event page", async () => {
  const summary = await diagnosticsApi.overview();
  vi.mocked(diagnosticsApi.overview).mockResolvedValue({
    ...summary,
    slowTraces: [
      { ...event, phase: "parse_total", status: "ok", duration_ms: 120 },
    ],
  });
  render(<DiagnosticsPage onTask={vi.fn()} />);
  fireEvent.click(screen.getByRole("tab", { name: "性能" }));
  fireEvent.change(screen.getByLabelText("Trace排序"), {
    target: { value: "slow" },
  });
  expect(
    await screen.findByRole("button", { name: new RegExp(trace) }),
  ).toBeTruthy();
});
it("only describes observed stages, with no timer-based connecting state", () => {
  const spawned: DiagnosticEvent = {
    ...event,
    phase: "download_first_progress",
    status: "started",
  };
  expect(observedStatus([spawned])).toBe("进程运行，等待首次有效进度");
  expect(observedStatus([{ ...spawned, status: "unobserved" }])).toBeNull();
  expect(
    observedStatus([
      { ...spawned, phase: "download_terminal", status: "cancelled" },
    ]),
  ).toBe("已取消");
  expect(observedStatus([spawned], "another-session")).toContain(
    "上一会话已结束",
  );
});
it("requests the bounded stage view for task details and still accepts older events", async () => {
  vi.mocked(diagnosticsApi.query).mockResolvedValue({
    events: [{ ...event, phase: "download_first_progress", status: "started" }],
    total: 1,
    nextCursor: null,
    unreadableLines: 0,
    truncated: false,
  });
  render(<TaskDiagnosticStatus taskId={task} active={false} />);
  expect(
    await screen.findByText("已观测阶段：进程运行，等待首次有效进度"),
  ).toBeTruthy();
  expect(diagnosticsApi.query).toHaveBeenCalledWith({
    taskId: task,
    traceId: undefined,
    stageView: true,
    limit: 100,
  });
  expect(screen.getByText(/尚无持续下载吞吐记录/)).toBeTruthy();
});
it("shows a truthful empty state and performs trusted report/directory actions", async () => {
  render(<DiagnosticsPage onTask={vi.fn()} />);
  expect(await screen.findByText("暂无诊断数据")).toBeTruthy();
  expect(screen.queryByText(/6.8s|100%/)).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "打开日志目录" }));
  await waitFor(() => expect(diagnosticsApi.open).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByRole("button", { name: "刷新报告" }));
  await waitFor(() => expect(diagnosticsApi.refresh).toHaveBeenCalledOnce());
});
it("shows missing progress without inventing a duration and locates the actual task", async () => {
  vi.mocked(diagnosticsApi.query).mockResolvedValue({
    events: [event],
    total: 1,
    nextCursor: null,
    unreadableLines: 0,
    truncated: false,
  });
  const locate = vi.fn();
  render(<DiagnosticsPage initialTask={task} onTask={locate} />);
  fireEvent.click(
    await screen.findByRole("button", { name: new RegExp(trace) }),
  );
  expect(await screen.findByText(/未观测到 · 未观测到结束耗时/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "定位任务" }));
  expect(locate).toHaveBeenCalledWith(task);
});
it("requires an explicit export preview and does not call export when cancelled", async () => {
  vi.mocked(diagnosticsApi.export).mockResolvedValue("本地保存完成");
  render(<DiagnosticsPage onTask={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "导出诊断包" }));
  expect(screen.getByText(/privacy-manifest.json/)).toBeTruthy();
  expect(diagnosticsApi.export).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "选择文件名并导出" }));
  await waitFor(() => expect(diagnosticsApi.export).toHaveBeenCalledOnce());
  expect(await screen.findByText("本地保存完成")).toBeTruthy();
});
it("sends bounded task, trace, level and keyword filters and paginates without full history", async () => {
  vi.mocked(diagnosticsApi.query).mockResolvedValue({
    events: [event],
    total: 250,
    nextCursor: 100,
    unreadableLines: 1,
    truncated: true,
  });
  render(<DiagnosticsPage onTask={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("搜索诊断"), {
    target: { value: "二次安全复核" },
  });
  fireEvent.change(screen.getByLabelText("筛选 Trace ID"), {
    target: { value: trace },
  });
  fireEvent.change(screen.getByLabelText("日志等级"), {
    target: { value: "warn" },
  });
  fireEvent.click(screen.getByRole("button", { name: "应用筛选" }));
  await waitFor(() =>
    expect(diagnosticsApi.query).toHaveBeenCalledWith(
      expect.objectContaining({
        search: "二次安全复核",
        traceId: trace,
        level: "warn",
        cursor: 0,
        limit: 100,
      }),
    ),
  );
  fireEvent.click(screen.getByRole("tab", { name: "事件" }));
  fireEvent.click(await screen.findByRole("button", { name: "下一页" }));
  await waitFor(() =>
    expect(diagnosticsApi.query).toHaveBeenCalledWith(
      expect.objectContaining({ cursor: 100 }),
    ),
  );
  expect(
    screen.queryByText(/FIXTURE_SECRET|PRIVATE_COOKIE|PRIVATE_PASSWORD/),
  ).toBeNull();
});
it("keeps diagnostic storage failures visible and never claims a successful export", async () => {
  vi.mocked(diagnosticsApi.health).mockResolvedValue({
    mode: "development",
    sessionId: trace,
    directory: null,
    writeRestricted: true,
    dropped: 2,
    criticalDropped: 1,
    generation: 1,
    level: "info",
  });
  vi.mocked(diagnosticsApi.open).mockRejectedValue(new Error("日志目录不可写"));
  render(<DiagnosticsPage onTask={vi.fn()} />);
  expect(await screen.findByText("日志写入受限 · 下载继续运行")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "打开日志目录" }));
  expect((await screen.findByRole("alert")).textContent).toBe("日志目录不可写");
});
it("renders throughput beside a Trace without turning low-frequency samples into completed spans", async () => {
  const sample: DiagnosticEvent = {
    ...event,
    event_id: "33333333-3333-4333-8333-333333333333",
    span_id: "44444444-4444-4444-8444-444444444444",
    phase: "download_throughput",
    event: "throughput_sample",
    status: "ok",
    fields: {
      ...event.fields,
      throughput: {
        observed_bytes: 1024,
        duration_ms: 5000,
        samples: 6,
        stream_count: 1,
        mean_bps: 204.8,
        window_5s_bps: 204.8,
        window_10s_bps: null,
        max_5s_bps: 204.8,
        stall_count: 0,
        stall_duration_ms: 0,
        progress_gap_ms: 0,
        final_sample: false,
      },
    },
  };
  vi.mocked(diagnosticsApi.query).mockResolvedValue({
    events: [sample, event],
    total: 2,
    nextCursor: null,
    unreadableLines: 0,
    truncated: false,
  });
  const result = render(
    <DiagnosticsPage initialTask={task} onTask={vi.fn()} />,
  );
  fireEvent.click(
    await screen.findByRole("button", { name: new RegExp(trace) }),
  );
  expect(
    await screen.findByRole("region", { name: "下载吞吐诊断" }),
  ).toBeTruthy();
  expect(screen.getByText("1.00 KiB")).toBeTruthy();
  expect(result.container.querySelectorAll(".diagnostic-span")).toHaveLength(1);
  expect(diagnosticsApi.query).toHaveBeenCalledWith({
    traceId: trace,
    stageView: true,
    limit: 200,
  });
  expect(
    vi
      .mocked(diagnosticsApi.query)
      .mock.calls.filter(([filter]) => filter?.limit === 100)
      .every(([filter]) => filter?.stageView === undefined),
  ).toBe(true);
  fireEvent.click(screen.getByRole("tab", { name: "事件" }));
  expect(await screen.findByText("采样有效")).toBeTruthy();
  expect(screen.getByText("5 秒窗口 205 B/s")).toBeTruthy();
});
