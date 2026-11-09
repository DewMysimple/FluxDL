import { afterEach, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import {
  diagnosticStatusLabel,
  latestThroughput,
  TaskThroughput,
  throughputBytes,
  throughputSpeed,
} from "../features/diagnostics/TaskThroughput";
import { observedStatus } from "../features/diagnostics/TaskDiagnosticStatus";
import type {
  DiagnosticEvent,
  ThroughputSnapshot,
} from "../features/diagnostics/types";

const snapshot: ThroughputSnapshot = {
  observed_bytes: 2 * 1024 * 1024,
  duration_ms: 8000,
  samples: 5,
  stream_count: 1,
  mean_bps: 256 * 1024,
  window_5s_bps: 128 * 1024,
  window_10s_bps: null,
  max_5s_bps: 256 * 1024,
  stall_count: 1,
  stall_duration_ms: 2000,
  progress_gap_ms: 1000,
  final_sample: false,
};
const event: DiagnosticEvent = {
  schema_version: 1,
  event_id: "11111111-1111-4111-8111-111111111111",
  session_id: "22222222-2222-4222-8222-222222222222",
  trace_id: "33333333-3333-4333-8333-333333333333",
  span_id: "44444444-4444-4444-8444-444444444444",
  parent_span_id: null,
  task_id: "55555555-5555-4555-8555-555555555555",
  timestamp_utc: "2026-11-09T13:00:00Z",
  elapsed_since_session_start_ms: 12_000,
  level: "info",
  source: "rust",
  event: "throughput_sample",
  phase: "download_throughput",
  status: "ok",
  duration_ms: null,
  fields: {
    related_trace_id: null,
    attempt_id: "66666666-6666-4666-8666-666666666666",
    error_code: null,
    throughput: snapshot,
  },
};

afterEach(cleanup);

it("uses explicit binary byte units and preserves unknown versus measured zero", () => {
  expect(throughputSpeed(1024)).toBe("1.00 KiB/s");
  expect(throughputSpeed(1024 * 1024)).toBe("1.00 MiB/s");
  expect(throughputSpeed(0)).toBe("0 B/s");
  expect(throughputBytes(2 * 1024 * 1024)).toBe("2.00 MiB");
  for (const value of [null, undefined, -1, NaN, Infinity])
    expect(throughputSpeed(value)).toBe("未测量");
});

it("does not carry throughput into a newer retry Trace or attempt", () => {
  expect(latestThroughput([event])).toBe(event);
  const newer = {
    ...event,
    event: "stage_started" as const,
    phase: "download_engine_spawn" as const,
    fields: { ...event.fields, throughput: undefined },
  };
  expect(
    latestThroughput([{ ...newer, trace_id: "new-trace" }, event]),
  ).toBeUndefined();
  expect(
    latestThroughput([
      { ...newer, fields: { ...newer.fields, attempt_id: "new-attempt" } },
      event,
    ]),
  ).toBeUndefined();
  expect(latestThroughput([newer, event])).toBe(event);
});

it("keeps successful throughput samples separate from task completion", () => {
  expect(diagnosticStatusLabel(event)).toBe("采样有效");
  expect(observedStatus([event])).toBeNull();
  expect(
    observedStatus([
      event,
      {
        ...event,
        event: "stage_started",
        phase: "download_active",
        status: "started",
      },
    ]),
  ).toBe("下载中");
});

it("shows actual low-frequency windows, observed bytes and bounded stall evidence", () => {
  render(<TaskThroughput event={event} sessionId={event.session_id} />);
  expect(screen.getByRole("region", { name: "下载吞吐诊断" })).toBeTruthy();
  expect(screen.getByText("128.00 KiB/s")).toBeTruthy();
  expect(screen.getByText("256.00 KiB/s")).toBeTruthy();
  expect(screen.getByText("2.00 MiB")).toBeTruthy();
  expect(screen.getByText("8.0 秒")).toBeTruthy();
  expect(screen.getByText("1 次 / 2.0 秒")).toBeTruthy();
  expect(screen.getAllByText("未测量")).toHaveLength(4);
  expect(screen.getByText(/进度间隔不代表网络断开/)).toBeTruthy();
  expect(screen.queryByText(/100%|KB\/s|kb\/s/)).toBeNull();
});

it("preserves final cancellation and historical session labels without inventing newer values", () => {
  const cancelled: DiagnosticEvent = {
    ...event,
    status: "cancelled",
    fields: {
      ...event.fields,
      throughput: { ...snapshot, final_sample: true },
    },
  };
  const result = render(
    <TaskThroughput event={cancelled} sessionId="new-session" />,
  );
  expect(screen.getByText(/上一会话记录 · 最终采样 · 已取消/)).toBeTruthy();
  result.rerender(<TaskThroughput event={cancelled} />);
  expect(screen.getByText("8.0 秒")).toBeTruthy();
  expect(screen.getByText("2.00 MiB")).toBeTruthy();
});

it("accepts older diagnostic events with no throughput fields as unmeasured", () => {
  const older: DiagnosticEvent = {
    ...event,
    phase: "download_active",
    event: "stage_started",
    fields: { related_trace_id: null, attempt_id: null, error_code: null },
  };
  expect(latestThroughput([older])).toBeUndefined();
  render(<TaskThroughput />);
  expect(screen.getByText(/尚无持续下载吞吐记录/)).toBeTruthy();
  expect(screen.queryByText("0 B/s")).toBeNull();
});

it("shows only whitelisted protocol and configuration evidence without asserting the network route", () => {
  const known: DiagnosticEvent = {
    ...event,
    fields: {
      ...event.fields,
      throughput: {
        ...snapshot,
        protocol: "http_dash_segments",
        downloader: "ytdlp_default",
        network_mode: "inherited",
      },
    },
  };
  const result = render(<TaskThroughput event={known} />);
  expect(screen.getByText("DASH 分片")).toBeTruthy();
  expect(screen.getByText("yt-dlp 默认选择")).toBeTruthy();
  expect(screen.getByText("继承网络环境（实际路由未确认）")).toBeTruthy();
  result.rerender(
    <TaskThroughput
      event={{
        ...known,
        fields: {
          ...known.fields,
          throughput: { ...snapshot, protocol: "unknown" },
        },
      }}
    />,
  );
  expect(screen.queryByText("DASH 分片")).toBeNull();
  expect(screen.queryByText("yt-dlp 默认选择")).toBeNull();
  expect(screen.getAllByText("未测量")).toHaveLength(4);
});
