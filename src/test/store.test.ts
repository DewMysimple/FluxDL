import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../lib/api";
import { createAppStore, toSnapshot } from "../stores/app";
let store: ReturnType<typeof createAppStore>;
import { snapshotFixture, taskFixture } from "./fixtures";

vi.mock("../lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("../lib/api")>();
  return { ...original, api: { ...original.api, snapshot: vi.fn() } };
});

beforeEach(() => {
  vi.clearAllMocks();
  store = createAppStore();
});
afterEach(() => store.getState().beginSync(async () => undefined)());

describe("authoritative full snapshot cache", () => {
  it("starts empty and finishes loading from a backend snapshot", async () => {
    expect(toSnapshot(store.getState()).tasks).toEqual([]);
    expect(store.getState().loading).toBe(true);
    const state = snapshotFixture([taskFixture("new")]);
    vi.mocked(api.snapshot).mockResolvedValue(state);
    await store.getState().refresh();
    expect(store.getState().loading).toBe(false);
    expect(toSnapshot(store.getState()).tasks.map((t) => t.id)).toEqual([
      "new",
    ]);
  });

  it("reflects task creation, measured progress and a final state without duplicating records", () => {
    const task = taskFixture("one");
    store.getState().setSnapshot(snapshotFixture([task]));
    store.getState().setSnapshot(
      snapshotFixture([
        {
          ...task,
          status: "downloading",
          progress: 24,
          downloadedBytes: 240,
        },
      ]),
    );
    expect(toSnapshot(store.getState()).tasks).toHaveLength(1);
    expect(toSnapshot(store.getState()).tasks[0]).toMatchObject({
      id: "one",
      status: "downloading",
      progress: 24,
      downloadedBytes: 240,
    });
    store.getState().setSnapshot(
      snapshotFixture([
        {
          ...task,
          status: "completed",
          finalPath: "C:\\Downloads\\one.mp4",
          fileExists: true,
        },
      ]),
    );
    expect(toSnapshot(store.getState()).tasks[0]).toMatchObject({
      status: "completed",
      fileExists: true,
    });
  });

  it("removes deleted backend records and keeps the backend task order", () => {
    const first = taskFixture("first");
    const second = taskFixture("second");
    store.getState().setSnapshot(snapshotFixture([second, first]));
    expect(toSnapshot(store.getState()).tasks.map((t) => t.id)).toEqual([
      "second",
      "first",
    ]);
    store.getState().setSnapshot(snapshotFixture([second]));
    expect(toSnapshot(store.getState()).tasks.map((t) => t.id)).toEqual([
      "second",
    ]);
    store.getState().setSnapshot(snapshotFixture());
    expect(toSnapshot(store.getState()).tasks).toEqual([]);
  });

  it("retains the last usable data and exposes an IPC refresh failure", async () => {
    const state = snapshotFixture([taskFixture("retained")]);
    store.getState().setSnapshot(state);
    vi.mocked(api.snapshot).mockRejectedValue(new Error("无法读取本地任务"));
    await store.getState().refresh();
    expect(toSnapshot(store.getState()).tasks.map((t) => t.id)).toEqual([
      "retained",
    ]);
    expect(store.getState().error).toContain("无法读取本地任务");
    expect(store.getState().loading).toBe(false);
    store.getState().clearError();
    expect(store.getState().error).toBeNull();
  });
});
