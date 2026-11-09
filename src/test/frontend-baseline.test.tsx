import { existsSync, writeFileSync } from "node:fs";
import { platform, release, totalmem } from "node:os";
import { resolve } from "node:path";
import { Profiler, type ProfilerOnRenderCallback } from "react";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "../lib/api";
import type { AppEvent, AppSnapshot } from "../lib/types";
import { useApp } from "../stores/app";
import {
  installMatchMedia,
  seedApp,
  snapshotFixture,
  taskFixture,
} from "./fixtures";
import App from "../App";

// Each History TaskRow renders this icon once. Retain its real implementation.
const counters = vi.hoisted(() => ({ taskRowRenders: 0 }));
vi.mock("lucide-react", async (importOriginal) => {
  const icons = await importOriginal<typeof import("lucide-react")>();
  const { createElement } = await import("react");
  return {
    ...icons,
    MoreHorizontal: (props: import("lucide-react").LucideProps) => {
      counters.taskRowRenders++;
      return createElement(icons.MoreHorizontal, props);
    },
  };
});
vi.mock("../lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("../lib/api")>();
  return { ...original, api: { ...original.api, snapshot: vi.fn() } };
});

function statistics(values: number[]) {
  const sorted = [...values].sort((a, b) => a - b);
  return {
    samples: values.length,
    p50: sorted[Math.ceil(sorted.length * 0.5) - 1],
    p95: sorted[Math.ceil(sorted.length * 0.95) - 1],
    min: sorted[0],
    max: sorted[sorted.length - 1],
  };
}
function progressSnapshot(seed: AppSnapshot, index: number): AppSnapshot {
  const snapshot: AppSnapshot = JSON.parse(JSON.stringify(seed));
  snapshot.revision = index;
  snapshot.tasks[0] = {
    ...snapshot.tasks[0],
    progress: index,
    downloadedBytes: index * 1024,
  };
  return snapshot;
}
function progressEvent(seed: AppSnapshot, index: number): AppEvent {
  const task = seed.tasks[0];
  return {
    sessionId: seed.sessionId,
    revision: index,
    change: {
      type: "taskProgress",
      taskId: task.id,
      status: "downloading",
      progress: index,
      speed: task.speed,
      eta: task.eta,
      downloadedBytes: index * 1024,
      totalBytes: task.totalBytes,
      updatedAt: task.updatedAt,
      logsAppend: [],
    },
  };
}
async function measureReact(
  seed: AppSnapshot,
  prepare: (index: number) => () => void,
  expectedRows: number,
) {
  seedApp(seed);
  vi.mocked(api.snapshot).mockResolvedValue(seed);
  const commits: number[] = [];
  const onRender: ProfilerOnRenderCallback = (_id, _phase, actualDuration) => {
    commits.push(actualDuration);
  };
  const view = render(
    <Profiler id="frontend-benchmark" onRender={onRender}>
      <App />
    </Profiler>,
  );
  await act(async () => {
    await Promise.resolve();
  });
  fireEvent.click(screen.getByRole("button", { name: /^下载历史$/ }));
  const warmup = prepare(1);
  await act(async () => warmup());
  const reactTimes: number[] = [],
    elapsedTimes: number[] = [],
    taskRowRenders: number[] = [],
    commitCounts: number[] = [];
  for (let index = 2; index <= 6; index++) {
    const apply = prepare(index); // Decode/build outside the timed update.
    counters.taskRowRenders = 0;
    commits.length = 0;
    const start = performance.now();
    await act(async () => apply());
    elapsedTimes.push(performance.now() - start);
    reactTimes.push(commits.reduce((sum, duration) => sum + duration, 0));
    taskRowRenders.push(counters.taskRowRenders);
    commitCounts.push(commits.length);
  }
  expect(taskRowRenders).toEqual(Array(5).fill(expectedRows));
  view.unmount();
  cleanup();
  return {
    reactActualDurationMs: statistics(reactTimes),
    updateThroughEffectsMs: statistics(elapsedTimes),
    unrelatedHistoryRowRendersPerUpdate: taskRowRenders,
    reactCommitCounts: commitCounts,
  };
}

afterEach(() => {
  cleanup();
  useApp.getState().beginSync(async () => undefined)();
});
it.runIf(process.env.FLUXDL_FRONTEND_BASELINE === "1")(
  "records separate full-snapshot and incremental-event frontend workloads",
  { timeout: 180_000 },
  async () => {
    installMatchMedia();
    const phase = process.env.FLUXDL_FRONTEND_PHASE ?? "before-refactor";
    const output = resolve(
      process.env.FLUXDL_FRONTEND_REPORT ??
        "docs/engineering/frontend-baseline.json",
    );
    if (
      [
        resolve("docs/engineering/frontend-baseline.json"),
        resolve("docs/engineering/frontend-stage2.json"),
      ].includes(output) &&
      existsSync(output)
    )
      throw new Error(
        "Recorded before/stage-2 evidence is immutable; set a new FLUXDL_FRONTEND_REPORT and phase.",
      );
    const measurements = [],
      deltaMeasurements = [];
    for (const count of [100, 1000, 5000]) {
      const seed = snapshotFixture(
        Array.from({ length: count }, (_, index) =>
          taskFixture(`benchmark-${index}`, {
            status: index === 0 ? "downloading" : "completed",
            updatedAt: `2026-01-${String((index % 28) + 1).padStart(2, "0")}T00:00:00Z`,
          }),
        ),
      );
      seedApp(seed);
      let notifications = 0;
      let unsubscribe = useApp.subscribe(() => {
        notifications++;
      });
      const fullTimes: number[] = [],
        fullSerialization: number[] = [];
      for (let index = 1; index <= 20; index++) {
        const input = progressSnapshot(seed, index);
        let start = performance.now();
        JSON.stringify(input);
        fullSerialization.push(performance.now() - start);
        start = performance.now();
        useApp.getState().setSnapshot(input);
        fullTimes.push(performance.now() - start);
      }
      unsubscribe();
      expect(notifications).toBe(20);
      const fullReact = await measureReact(
        seed,
        (index) => {
          const input = progressSnapshot(seed, index);
          return () => useApp.getState().setSnapshot(input);
        },
        phase === "before-refactor" ? count - 1 : 0,
      );
      measurements.push({
        workload: "fullSnapshot",
        taskCount: count,
        visibleHistoryRows: count - 1,
        changedDomainTasks: 1,
        recreatedTaskRecordsPerSnapshot: count,
        snapshotPayloadBytes: Buffer.byteLength(JSON.stringify(seed), "utf8"),
        storeNotificationsFor20Updates: notifications,
        storeUpdateMs: statistics(fullTimes),
        snapshotSerializeMs: statistics(fullSerialization),
        ...fullReact,
      });

      seedApp(seed);
      notifications = 0;
      unsubscribe = useApp.subscribe(() => {
        notifications++;
      });
      const deltaTimes: number[] = [],
        deltaSerialization: number[] = [];
      for (let index = 1; index <= 20; index++) {
        const input = progressEvent(seed, index);
        let start = performance.now();
        JSON.stringify(input);
        deltaSerialization.push(performance.now() - start);
        start = performance.now();
        useApp.getState().applyEvent(input);
        deltaTimes.push(performance.now() - start);
      }
      unsubscribe();
      expect(notifications).toBe(20);
      const deltaReact = await measureReact(
        seed,
        (index) => {
          const input = progressEvent(seed, index);
          return () => useApp.getState().applyEvent(input);
        },
        0,
      );
      deltaMeasurements.push({
        workload: "taskProgressEvent",
        taskCount: count,
        visibleHistoryRows: count - 1,
        changedDomainTasks: 1,
        recreatedTaskRecordsPerEvent: 1,
        eventPayloadBytes: Buffer.byteLength(
          JSON.stringify(progressEvent(seed, 1)),
          "utf8",
        ),
        storeNotificationsFor20Updates: notifications,
        storeApplyEventMs: statistics(deltaTimes),
        eventSerializeMs: statistics(deltaSerialization),
        ...deltaReact,
      });
    }
    const report = {
      schemaVersion: 2,
      phase,
      generatedAt: new Date().toISOString(),
      environment: {
        node: process.version,
        platform: platform(),
        osRelease: release(),
        totalMemoryBytes: totalmem(),
        renderer:
          "ReactDOM development build + jsdom; no StrictMode for benchmark",
      },
      method: {
        storeSamples: 20,
        reactSamples: 5,
        warmupReactUpdates: 1,
        changedTask:
          "one active task while History displays only completed tasks",
        taskRowCounter:
          "transparent MoreHorizontal wrapper; History renders it once per TaskRow",
        decodingOutsideTimedUpdate: true,
        quantiles: "nearest-rank",
        workloads: {
          measurements:
            "complete snapshots, same task fixture as before/stage-2 with sessionId/revision protocol fields added",
          deltaMeasurements:
            "actual production applyEvent/reducer with exact taskProgress envelope; synthetic local fixture, no mocked reducer and no transport timing",
        },
        limitations: [
          "jsdom has no native layout, painting, WebView2 IPC transport or FPS measurement",
          "React Profiler adds instrumentation overhead; comparative development timings",
          "minimal metadata and empty logs are a controlled lower-bound fixture, not production telemetry",
          "full snapshots recreate every task; deltas recreate only the affected task, and these workloads are reported separately",
          "five React samples are a small comparative sample; p95 is the maximum observed sample",
          "immutable Record replacement still shallow-copies tasksById; not a constant-time persistent data structure",
        ],
      },
      measurements,
      deltaMeasurements,
    };
    writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
    console.info(
      JSON.stringify(
        {
          report: process.env.FLUXDL_FRONTEND_REPORT,
          full: measurements.map((item) => ({
            count: item.taskCount,
            p50: item.updateThroughEffectsMs.p50,
          })),
          delta: deltaMeasurements.map((item) => ({
            count: item.taskCount,
            bytes: item.eventPayloadBytes,
            p50: item.updateThroughEffectsMs.p50,
          })),
        },
        null,
        2,
      ),
    );
  },
);
