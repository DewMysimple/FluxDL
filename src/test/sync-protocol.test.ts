import { afterEach, describe, expect, it, vi } from "vitest";
import { createAppStore, toSnapshot } from "../stores/app";
import type {
  AppChange,
  AppEvent,
  AppSnapshot,
  DownloadTask,
} from "../lib/types";
import { snapshotFixture, taskFixture } from "./fixtures";

function deferred<T>() {
  let complete: (value: T) => void = () => {
    throw new Error("Not initialized");
  };
  const promise = new Promise<T>((resolve) => {
    complete = resolve;
  });
  return { promise, resolve: (value: T) => complete(value) };
}
const snapshot = (
  revision: number,
  tasks: DownloadTask[] = [taskFixture("a", { status: "downloading" })],
  sessionId = "s1",
): AppSnapshot => ({ ...snapshotFixture(tasks), sessionId, revision });
const event = (
  revision: number,
  change: AppChange,
  sessionId = "s1",
): AppEvent => ({ sessionId, revision, change });
const progress = (
  revision: number,
  value = revision,
  logsAppend: string[] = [],
): AppEvent =>
  event(revision, {
    type: "taskProgress",
    taskId: "a",
    status: "downloading",
    progress: value,
    speed: 1000,
    eta: 10,
    downloadedBytes: value * 1000,
    totalBytes: 100000,
    updatedAt: "2026-01-02T00:00:00Z",
    logsAppend,
  });
const cleanup: Array<() => void> = [];
function cache(
  initial = snapshot(0),
  load: () => Promise<AppSnapshot> = () => new Promise(() => {}),
  limits?: { maxEvents?: number; maxBytes?: number },
) {
  const store = createAppStore(load, limits);
  store.getState().setSnapshot(initial);
  cleanup.push(() => store.getState().beginSync(async () => undefined)());
  return store;
}
afterEach(() => {
  cleanup.splice(0).forEach((dispose) => dispose());
  vi.useRealTimers();
});

describe("session/revision recovery", () => {
  it("does not let an old in-flight snapshot erase a malformed-event recovery hint", async () => {
    const first = deferred<AppSnapshot>();
    const load = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce(snapshot(1));
    const store = cache(snapshot(0), load);
    const pending = store.getState().refresh();
    store.getState().applyEvent({
      sessionId: "s1",
      revision: 1,
      change: { type: "futureChange" },
    });
    first.resolve(snapshot(0));
    await pending;
    await vi.waitFor(() => expect(store.getState().revision).toBe(1));
    expect(load).toHaveBeenCalledTimes(2);
    expect(store.getState().inspectSync().needsSnapshot).toBe(false);
  });

  it("recovers malformed/null/future envelopes without throwing or consuming their sequence", async () => {
    const load = deferred<AppSnapshot>();
    const store = cache(snapshot(0), () => load.promise);
    for (const value of [
      null,
      {},
      { sessionId: "s1", revision: 1, change: null },
      { sessionId: "s1", revision: 1, change: { type: "taskProgress" } },
      { sessionId: "s1", revision: 1, change: { type: "futureChange" } },
    ]) {
      expect(() => store.getState().applyEvent(value)).not.toThrow();
    }
    expect(store.getState().revision).toBe(0);
    expect(store.getState().error).toContain("格式无效");
    const request = store.getState().refresh();
    load.resolve(snapshot(1));
    await request;
    expect(store.getState().revision).toBe(1);
  });
  it("buffers out of order events, drains a filled gap, and ignores duplicates", async () => {
    const load = deferred<AppSnapshot>();
    const store = cache(snapshot(10), () => load.promise);
    store.getState().applyEvent(progress(12, 60, ["twelve"]));
    expect(store.getState().revision).toBe(10);
    store.getState().applyEvent(progress(11, 40, ["eleven"]));
    store.getState().applyEvent(progress(12, 99, ["duplicate"]));
    expect(store.getState().revision).toBe(12);
    expect(store.getState().tasksById.a).toMatchObject({
      progress: 60,
      logs: ["eleven", "twelve"],
    });
    const pending = store.getState().refresh();
    load.resolve(snapshot(10));
    await pending;
    expect(store.getState().revision).toBe(12);
    expect(store.getState().inspectSync().needsSnapshot).toBe(false);
  });

  it("accepts a newer snapshot received after an event, rather than discarding every in-flight response", async () => {
    const load = deferred<AppSnapshot>();
    const store = cache(snapshot(1), () => load.promise);
    const request = store.getState().refresh();
    store.getState().applyEvent(progress(2, 20));
    load.resolve(
      snapshot(5, [taskFixture("a", { status: "downloading", progress: 50 })]),
    );
    await request;
    expect(store.getState().revision).toBe(5);
    expect(store.getState().tasksById.a.progress).toBe(50);
  });

  it("replays events after the returned snapshot and discards those it already covers", async () => {
    const load = deferred<AppSnapshot>();
    const store = cache(snapshot(10), () => load.promise);
    store.getState().applyEvent(progress(13, 70, ["after-snapshot"]));
    const request = store.getState().refresh();
    load.resolve(
      snapshot(12, [
        taskFixture("a", { status: "downloading", logs: ["covered"] }),
      ]),
    );
    await request;
    expect(store.getState().revision).toBe(13);
    expect(store.getState().tasksById.a.logs).toEqual([
      "covered",
      "after-snapshot",
    ]);
  });

  it("switches sessions only on an authoritative snapshot and retires the old session", async () => {
    const load = deferred<AppSnapshot>();
    const store = cache(snapshot(99), () => load.promise);
    store
      .getState()
      .applyEvent(
        event(
          1,
          { type: "taskUpsert", task: taskFixture("new"), index: 0 },
          "s2",
        ),
      );
    expect(store.getState().sessionId).toBe("s1");
    const request = store.getState().refresh();
    load.resolve(snapshot(0, [], "s2"));
    await request;
    expect(store.getState().sessionId).toBe("s2");
    expect(store.getState().taskIds).toEqual(["new"]);
    store.getState().applyEvent(progress(100, 100));
    store.getState().setSnapshot(snapshot(101));
    expect(store.getState().sessionId).toBe("s2");
    expect(store.getState().revision).toBe(1);
  });

  it("does not reject a new-session hint that arrived after the current snapshot request started", async () => {
    const first = deferred<AppSnapshot>();
    const load = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce(snapshot(1, [taskFixture("new")], "s2"));
    const store = cache(snapshot(1), load);
    const request = store.getState().refresh();
    store.getState().applyEvent(event(1, { type: "checkpoint" }, "s2"));
    first.resolve(snapshot(1));
    await request;
    await vi.waitFor(() => expect(store.getState().sessionId).toBe("s2"));
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("rejects a foreign hint when a request started after that hint confirms the current session", async () => {
    const load = vi.fn().mockResolvedValue(snapshot(1));
    const store = cache(snapshot(1), load);
    store
      .getState()
      .applyEvent(event(2, { type: "checkpoint" }, "old-foreign"));
    await store.getState().refresh();
    expect(store.getState().sessionId).toBe("s1");
    expect(store.getState().inspectSync().bufferedEvents).toBe(0);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it("confirms earlier foreign hints without rolling back progress or repeating snapshots across three races", async () => {
    const responses: Array<ReturnType<typeof deferred<AppSnapshot>>> = [];
    const load = vi.fn(() => {
      const response = deferred<AppSnapshot>();
      responses.push(response);
      return response.promise;
    });
    const store = cache(snapshot(10), load);
    for (let round = 0; round < 3; round++) {
      const foreign = `old-foreign-${round}`;
      store.getState().applyEvent(event(1, { type: "checkpoint" }, foreign));
      const request = store.getState().refresh();
      await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(round + 1));
      store.getState().applyEvent(progress(11 + round, 60 + round));
      const advanced = store.getState();
      responses[round].resolve(snapshot(10 + round));
      await request;
      await Promise.resolve();
      expect(store.getState().revision).toBe(11 + round);
      expect(store.getState().tasksById).toBe(advanced.tasksById);
      expect(store.getState().tasksById.a.progress).toBe(60 + round);
      expect(store.getState().inspectSync()).toMatchObject({
        needsSnapshot: false,
        bufferedEvents: 0,
        inFlight: false,
      });
      expect(store.getState().inspectSync().retiredSessions).toContain(foreign);
      expect(load).toHaveBeenCalledTimes(round + 1);
    }
  });

  it("keeps a new foreign hint seen after a request whose snapshot is overtaken by progress", async () => {
    const first = deferred<AppSnapshot>();
    const load = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce(snapshot(1, [], "s2"));
    const store = cache(snapshot(10), load);
    const request = store.getState().refresh();
    store.getState().applyEvent(progress(11));
    store.getState().applyEvent(event(1, { type: "checkpoint" }, "s2"));
    first.resolve(snapshot(10));
    await request;
    await vi.waitFor(() => expect(store.getState().sessionId).toBe("s2"));
    expect(store.getState().inspectSync().retiredSessions).not.toContain("s2");
    expect(store.getState().inspectSync().needsSnapshot).toBe(false);
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("keeps a repeated foreign-session observation made during an earlier request", async () => {
    const first = deferred<AppSnapshot>();
    const load = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce(snapshot(1, [], "s2"));
    const store = cache(snapshot(10), load);
    store.getState().applyEvent(event(1, { type: "checkpoint" }, "s2"));
    const request = store.getState().refresh();
    store.getState().applyEvent(progress(11));
    store.getState().applyEvent(event(2, { type: "checkpoint" }, "s2"));
    first.resolve(snapshot(10));
    await request;
    await vi.waitFor(() => expect(store.getState().sessionId).toBe("s2"));
    expect(store.getState().revision).toBe(2);
    expect(store.getState().inspectSync().retiredSessions).not.toContain("s2");
    expect(store.getState().inspectSync().needsSnapshot).toBe(false);
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("keeps the current-session dropped revision watermark after an older snapshot excludes a foreign hint", async () => {
    const first = deferred<AppSnapshot>();
    const second = deferred<AppSnapshot>();
    const load = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => second.promise)
      .mockResolvedValueOnce(snapshot(13));
    const store = cache(snapshot(10), load, { maxEvents: 1 });
    store
      .getState()
      .applyEvent(event(1, { type: "checkpoint" }, "old-foreign"));
    const request = store.getState().refresh();
    store.getState().applyEvent(progress(11));
    store.getState().applyEvent(progress(13));
    first.resolve(snapshot(10));
    await request;
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(2));
    expect(store.getState().revision).toBe(11);
    expect(store.getState().inspectSync().retiredSessions).toContain(
      "old-foreign",
    );
    expect(store.getState().inspectSync().needsSnapshot).toBe(true);
    second.resolve(snapshot(12));
    await vi.waitFor(() => expect(store.getState().revision).toBe(13));
    expect(load).toHaveBeenCalledTimes(3);
    expect(store.getState().inspectSync().needsSnapshot).toBe(false);
  });

  it("keeps overflow bounded and requests a snapshot covering the dropped revision watermark", async () => {
    const first = deferred<AppSnapshot>();
    const load = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce(snapshot(5));
    const store = cache(snapshot(0), load, {
      maxEvents: 2,
      maxBytes: 1_000_000,
    });
    store.getState().applyEvent(progress(3));
    store.getState().applyEvent(progress(4));
    store.getState().applyEvent(progress(5));
    expect(store.getState().inspectSync().bufferedEvents).toBeLessThanOrEqual(
      2,
    );
    const request = store.getState().refresh();
    first.resolve(snapshot(1));
    await request;
    await vi.waitFor(() => expect(store.getState().revision).toBe(5));
    expect(load).toHaveBeenCalledTimes(2);
    expect(store.getState().inspectSync().needsSnapshot).toBe(false);
  });

  it("bounds buffered bytes even for one oversized semantic packet", async () => {
    const load = deferred<AppSnapshot>();
    const large = taskFixture("large", { logs: ["x".repeat(2000)] });
    const store = cache(snapshot(0), () => load.promise, {
      maxEvents: 256,
      maxBytes: 128,
    });
    store
      .getState()
      .applyEvent(event(3, { type: "taskUpsert", task: large, index: 1 }));
    expect(store.getState().inspectSync().bufferedBytes).toBeLessThanOrEqual(
      128,
    );
    const request = store.getState().refresh();
    load.resolve(snapshot(3, [taskFixture("a"), large]));
    await request;
    expect(store.getState().tasksById.large.logs).toEqual(large.logs);
  });

  it("recovers after a first snapshot failure even when no more task events arrive", async () => {
    vi.useFakeTimers();
    const load = vi
      .fn()
      .mockRejectedValueOnce(new Error("暂时不可用"))
      .mockResolvedValueOnce(snapshot(4));
    const store = cache(snapshot(1), load);
    store.getState().applyEvent(progress(4));
    await vi.advanceTimersByTimeAsync(0);
    expect(store.getState().error).toContain("暂时不可用");
    expect(store.getState().revision).toBe(1);
    expect(store.getState().inspectSync().inFlight).toBe(false);
    await vi.advanceTimersByTimeAsync(250);
    expect(store.getState().revision).toBe(4);
    expect(store.getState().error).toBeNull();
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("stops automatic retries after its budget, but explicit recovery can try again", async () => {
    vi.useFakeTimers();
    const load = vi
      .fn()
      .mockRejectedValueOnce(new Error("first"))
      .mockRejectedValueOnce(new Error("second"))
      .mockResolvedValueOnce(snapshot(3));
    const store = cache(snapshot(0), load);
    store.getState().applyEvent(progress(3));
    await vi.runAllTimersAsync();
    expect(load).toHaveBeenCalledTimes(2);
    expect(store.getState().inspectSync().inFlight).toBe(false);
    await store.getState().refresh();
    expect(load).toHaveBeenCalledTimes(3);
    expect(store.getState().revision).toBe(3);
  });

  it("can recover from a failed request when a later checkpoint arrives before the failure retry", async () => {
    vi.useFakeTimers();
    const done = taskFixture("a", { status: "completed" });
    const load = vi
      .fn()
      .mockRejectedValueOnce(new Error("temporary"))
      .mockResolvedValueOnce(snapshot(4, [done]));
    const store = cache(snapshot(1), load);
    store.getState().applyEvent(progress(3));
    await vi.advanceTimersByTimeAsync(0);
    store.getState().applyEvent(event(4, { type: "checkpoint" }));
    await store.getState().refresh();
    expect(load).toHaveBeenCalledTimes(2);
    expect(store.getState().tasksById.a.status).toBe("completed");
    await vi.runAllTimersAsync();
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("ignores late requests and cancels failure retries after lifecycle disposal", async () => {
    const load = deferred<AppSnapshot>();
    const store = cache(snapshot(1), () => load.promise);
    const stop = store.getState().beginSync(async () => undefined);
    const request = store.getState().refresh();
    await Promise.resolve();
    stop();
    load.resolve(snapshot(2));
    await request;
    expect(store.getState().revision).toBe(1);
    vi.useFakeTimers();
    const reject = vi.fn().mockRejectedValue(new Error("unavailable"));
    const retrying = cache(snapshot(0), reject);
    retrying.getState().applyEvent(progress(3));
    await vi.advanceTimersByTimeAsync(0);
    retrying.getState().beginSync(async () => undefined)();
    await vi.runAllTimersAsync();
    expect(reject).toHaveBeenCalledTimes(1);
  });
});

describe("task order, lifecycle, logs, details and checkpoints", () => {
  it("moves a retried task to the final backend index without duplicates and removes by ID", () => {
    const store = cache(
      snapshot(
        0,
        ["a", "b", "c"].map((id) => taskFixture(id, { status: "failed" })),
      ),
    );
    const retry = event(1, {
      type: "taskUpsert",
      task: taskFixture("b"),
      index: 2,
    });
    store.getState().applyEvent(retry);
    store.getState().applyEvent(retry);
    expect(store.getState().taskIds).toEqual(["a", "c", "b"]);
    store.getState().applyEvent(event(2, { type: "taskRemoved", taskId: "c" }));
    store
      .getState()
      .applyEvent(event(3, { type: "taskRemoved", taskId: "already-missing" }));
    expect(store.getState().taskIds).toEqual(["a", "b"]);
  });

  it("resynchronizes invalid indices and unknown progress IDs instead of inventing partial tasks", async () => {
    const load = deferred<AppSnapshot>();
    const store = cache(snapshot(0), () => load.promise);
    store
      .getState()
      .applyEvent(
        event(1, { type: "taskUpsert", task: taskFixture("b"), index: 99 }),
      );
    store.getState().applyEvent({
      ...progress(2),
      change: {
        ...progress(2).change,
        type: "taskProgress",
        taskId: "missing",
        status: "downloading",
        progress: 1,
        speed: null,
        eta: null,
        downloadedBytes: 1,
        totalBytes: null,
        updatedAt: "",
        logsAppend: [],
      },
    });
    expect(store.getState().taskIds).toEqual(["a"]);
    const request = store.getState().refresh();
    load.resolve(snapshot(2, [taskFixture("a"), taskFixture("b")]));
    await request;
    expect(store.getState().taskIds).toEqual(["a", "b"]);
  });

  it("does not revive terminal tasks with a progress packet, but allows explicit retry and real stream resets", () => {
    const done = taskFixture("a", { status: "completed", progress: 100 });
    const store = cache(snapshot(0, [done]));
    store.getState().applyEvent(progress(1, 10));
    expect(store.getState().tasksById.a).toBe(done);
    expect(store.getState().revision).toBe(1);
    store.getState().applyEvent(
      event(2, {
        type: "taskUpsert",
        task: taskFixture("a", { status: "downloading", progress: 90 }),
        index: 0,
      }),
    );
    store.getState().applyEvent(progress(3, 10));
    expect(store.getState().tasksById.a.progress).toBe(10);
    store.getState().applyEvent(
      event(4, {
        type: "taskUpsert",
        task: taskFixture("a", { status: "postprocessing" }),
        index: 0,
      }),
    );
    store.getState().applyEvent(progress(5));
    expect(store.getState().tasksById.a.status).toBe("postprocessing");
    store.getState().applyEvent(
      event(6, {
        type: "taskUpsert",
        task: taskFixture("a", { status: "downloading", progress: 0 }),
        index: 0,
      }),
    );
    store.getState().applyEvent(progress(7, 3));
    expect(store.getState().tasksById.a.status).toBe("downloading");
    expect(store.getState().tasksById.a.progress).toBe(3);
  });

  it("does not duplicate snapshot-covered appends and preserves legitimate repeated log text after truncation", () => {
    const store = cache(
      snapshot(10, [
        taskFixture("a", { status: "downloading", logs: ["A", "B"] }),
      ]),
    );
    store.getState().setSnapshot(
      snapshot(12, [
        taskFixture("a", {
          status: "downloading",
          logs: ["A", "B", "C", "D"],
        }),
      ]),
    );
    store.getState().applyEvent(progress(11, 11, ["C", "D"]));
    store.getState().applyEvent(progress(13, 13, ["E"]));
    store.getState().applyEvent(progress(13, 13, ["E"]));
    expect(store.getState().tasksById.a.logs).toEqual([
      "A",
      "B",
      "C",
      "D",
      "E",
    ]);
    const full = Array.from({ length: 100 }, (_, index) => String(index + 1));
    store.getState().applyEvent(
      event(14, {
        type: "taskUpsert",
        task: taskFixture("a", { status: "downloading", logs: full }),
        index: 0,
      }),
    );
    store.getState().applyEvent(progress(15, 15, ["101", "102"]));
    expect(store.getState().tasksById.a.logs).toEqual([
      ...full.slice(2),
      "101",
      "102",
    ]);
    store
      .getState()
      .applyEvent(progress(16, 16, ["same warning", "same warning"]));
    expect(store.getState().tasksById.a.logs).toHaveLength(100);
    expect(store.getState().tasksById.a.logs.slice(-2)).toEqual([
      "same warning",
      "same warning",
    ]);
  });

  it("shares unrelated tasks/settings/order for progress and compact file availability", () => {
    const store = cache(
      snapshot(0, [
        taskFixture("a", { status: "downloading" }),
        taskFixture("b", { status: "completed" }),
      ]),
    );
    const before = store.getState();
    store.getState().applyEvent(progress(1));
    expect(store.getState().tasksById.b).toBe(before.tasksById.b);
    expect(store.getState().taskIds).toBe(before.taskIds);
    expect(store.getState().settings).toBe(before.settings);
    store.getState().applyEvent(
      event(2, {
        type: "fileAvailabilityUpdated",
        files: [{ taskId: "b", fileExists: true }],
      }),
    );
    expect(store.getState().tasksById.b.fileExists).toBe(true);
    expect(store.getState().listingRevision).toBe(before.listingRevision);
    const tasks = store.getState().tasksById;
    store.getState().applyEvent(
      event(3, {
        type: "settingsUpdated",
        settings: { ...before.settings, theme: "light" },
      }),
    );
    store.getState().applyEvent(
      event(4, {
        type: "diagnosticsUpdated",
        engines: { ...before.engines, warnings: ["diagnostic"] },
      }),
    );
    expect(store.getState().tasksById).toBe(tasks);
    expect(store.getState().settings.theme).toBe("light");
  });

  it("merges only same-version task details and requests a full snapshot for newer details", async () => {
    const load = deferred<AppSnapshot>();
    const store = cache(snapshot(3), () => load.promise);
    store.getState().mergeDetails({
      sessionId: "s1",
      revision: 3,
      task: taskFixture("a", { status: "downloading", logs: ["details"] }),
    });
    expect(store.getState().tasksById.a.logs).toEqual(["details"]);
    store.getState().mergeDetails({
      sessionId: "s1",
      revision: 2,
      task: taskFixture("a", { status: "failed" }),
    });
    expect(store.getState().tasksById.a.status).toBe("downloading");
    store.getState().mergeDetails({
      sessionId: "s1",
      revision: 4,
      task: taskFixture("a", { status: "completed" }),
    });
    expect(store.getState().revision).toBe(3);
    const request = store.getState().refresh();
    load.resolve(snapshot(4, [taskFixture("a", { status: "completed" })]));
    await request;
    expect(store.getState().tasksById.a.status).toBe("completed");
  });

  it("does not let an older in-flight snapshot satisfy a newer detail observation", async () => {
    const first = deferred<AppSnapshot>();
    const done = taskFixture("a", { status: "completed" });
    const load = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce(snapshot(1))
      .mockResolvedValueOnce(snapshot(2, [done]));
    const store = cache(snapshot(0), load);
    const request = store.getState().refresh();
    store.getState().mergeDetails({ sessionId: "s1", revision: 2, task: done });
    first.resolve(snapshot(0));
    await request;
    await vi.waitFor(() => expect(store.getState().revision).toBe(2));
    expect(store.getState().tasksById.a.status).toBe("completed");
    expect(load).toHaveBeenCalledTimes(3);
    expect(store.getState().inspectSync().needsSnapshot).toBe(false);
  });

  it("uses a healthy checkpoint only to advance the version, preserving all domain references", () => {
    const store = cache(snapshot(2));
    const before = store.getState();
    store.getState().applyEvent(event(3, { type: "checkpoint" }));
    const after = store.getState();
    expect(after.revision).toBe(3);
    expect(after.tasksById).toBe(before.tasksById);
    expect(after.taskIds).toBe(before.taskIds);
    expect(after.settings).toBe(before.settings);
    expect(after.engines).toBe(before.engines);
    expect(after.listingRevision).toBe(before.listingRevision);
  });

  it("recovers a missing final completion from only a later checkpoint and converges two windows", async () => {
    const completed = taskFixture("a", {
      status: "completed",
      finalPath: "C:\\Downloads\\a.mp4",
    });
    const workspace = cache(snapshot(1));
    const load = vi.fn().mockResolvedValue(snapshot(3, [completed]));
    const quick = cache(snapshot(1), load);
    workspace
      .getState()
      .applyEvent(event(2, { type: "taskUpsert", task: completed, index: 0 }));
    workspace.getState().applyEvent(event(3, { type: "checkpoint" }));
    quick.getState().applyEvent(event(3, { type: "checkpoint" }));
    await quick.getState().refresh();
    expect(toSnapshot(quick.getState())).toEqual(
      toSnapshot(workspace.getState()),
    );
    expect(quick.getState().tasksById.a.status).toBe("completed");
    expect(load).toHaveBeenCalledTimes(1);
  });

  it("does not erase an unrelated user-operation error when synchronization succeeds", async () => {
    const store = cache(snapshot(1), async () => snapshot(3));
    store.getState().notify("保存目录不可写");
    store.getState().applyEvent(progress(3));
    await store.getState().refresh();
    expect(store.getState().error).toBe("保存目录不可写");
  });
});
