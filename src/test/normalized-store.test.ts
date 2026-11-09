import { describe, expect, it } from "vitest";
import { createAppStore, toSnapshot } from "../stores/app";
import { createTaskListingSelector, selectTaskIds } from "../stores/selectors";
import type { AppSnapshot } from "../lib/types";
import { mediaFixture, snapshotFixture, taskFixture } from "./fixtures";

function deferred<T>() {
  let fulfill: (value: T) => void = () => {
    throw new Error("Promise not initialized");
  };
  const promise = new Promise<T>((resolve) => {
    fulfill = resolve;
  });
  return { promise, resolve: (value: T) => fulfill(value) };
}

describe("normalized task cache and selectors", () => {
  it("keeps unchanged task/settings/engine references and listing membership during progress", () => {
    const store = createAppStore();
    const first = snapshotFixture([
      taskFixture("active", { status: "downloading" }),
      taskFixture("history", { status: "completed" }),
    ]);
    store.getState().setSnapshot(first);
    const previous = store.getState();
    const historyIds = createTaskListingSelector("history");
    const beforeIds = historyIds(previous);
    const next = snapshotFixture(
      first.tasks.map((task) =>
        task.id === "active"
          ? {
              ...task,
              downloadedBytes: 1024,
              progress: 20,
              updatedAt: "2026-01-02T00:00:00Z",
            }
          : { ...task, media: { ...task.media }, logs: [] },
      ),
    );
    store.getState().setSnapshot(next);
    const current = store.getState();
    expect(current.tasksById.active).not.toBe(previous.tasksById.active);
    expect(current.tasksById.history).toBe(previous.tasksById.history);
    expect(current.settings).toBe(previous.settings);
    expect(current.engines).toBe(previous.engines);
    expect(current.taskIds).toBe(previous.taskIds);
    expect(current.listingRevision).toBe(previous.listingRevision);
    expect(historyIds(current)).toBe(beforeIds);
  });

  it("invalidates membership, completed ordering and searchable metadata when they change", () => {
    const store = createAppStore();
    const older = taskFixture("older", { status: "completed" });
    const next = taskFixture("next");
    store.getState().setSnapshot(snapshotFixture([older, next]));
    const history = createTaskListingSelector("history");
    expect(history(store.getState())).toEqual(["older"]);
    store
      .getState()
      .setSnapshot(
        snapshotFixture([
          older,
          { ...next, status: "completed", updatedAt: "2026-01-03T00:00:00Z" },
        ]),
      );
    expect(history(store.getState())).toEqual(["next", "older"]);
    store.getState().setSnapshot(
      snapshotFixture([
        {
          ...older,
          media: mediaFixture({
            title: "New searchable title",
            source: "Archive",
          }),
          updatedAt: "2026-01-04T00:00:00Z",
        },
        { ...next, status: "completed", updatedAt: "2026-01-03T00:00:00Z" },
      ]),
    );
    expect(history(store.getState())).toEqual(["older", "next"]);
    expect(selectTaskIds(store.getState(), "history", "ARCHIVE")).toEqual([
      "older",
    ]);
    store.getState().setSnapshot(snapshotFixture([next]));
    expect(store.getState().tasksById.older).toBeUndefined();
    expect(history(store.getState())).toEqual([]);
  });

  it("treats an identical full snapshot as a no-op notification", () => {
    const store = createAppStore();
    const snapshot = snapshotFixture([taskFixture("same")]);
    store.getState().setSnapshot(snapshot);
    let notifications = 0;
    const dispose = store.subscribe(() => {
      notifications++;
    });
    store
      .getState()
      .setSnapshot(
        snapshotFixture(snapshot.tasks.map((task) => ({ ...task }))),
      );
    dispose();
    expect(notifications).toBe(0);
  });

  it("lets two independent window caches converge without sharing mutable state", () => {
    const workspace = createAppStore();
    const quick = createAppStore();
    const initial = snapshotFixture([taskFixture("one")]);
    workspace.getState().setSnapshot(initial);
    quick.getState().setSnapshot(initial);
    const progress = snapshotFixture([
      taskFixture("one", { status: "downloading", progress: 42 }),
    ]);
    workspace.getState().setSnapshot(progress);
    expect(quick.getState().tasksById.one.status).toBe("queued");
    quick.getState().setSnapshot(progress);
    expect(toSnapshot(workspace.getState())).toEqual(
      toSnapshot(quick.getState()),
    );
  });
});

describe("local request ordering with the existing snapshot protocol", () => {
  it("does not let a pending request overwrite a newer subscribed snapshot", async () => {
    const request = deferred<AppSnapshot>();
    const store = createAppStore(() => request.promise);
    const refreshing = store.getState().refresh();
    store.getState().setSnapshot({
      ...snapshotFixture([taskFixture("new-event")]),
      revision: 2,
    });
    request.resolve({
      ...snapshotFixture([taskFixture("old-response")]),
      revision: 1,
    });
    await refreshing;
    expect(store.getState().taskIds).toEqual(["new-event"]);
  });

  it("coalesces concurrent refreshes and permits another request after completion", async () => {
    const first = deferred<AppSnapshot>();
    const second = deferred<AppSnapshot>();
    let calls = 0;
    const store = createAppStore(() =>
      ++calls === 1 ? first.promise : second.promise,
    );
    const one = store.getState().refresh();
    const shared = store.getState().refresh();
    await Promise.resolve();
    expect(calls).toBe(1);
    first.resolve({ ...snapshotFixture([taskFixture("first")]), revision: 1 });
    await Promise.all([one, shared]);
    const next = store.getState().refresh();
    store.getState().setSnapshot({
      ...snapshotFixture([taskFixture("latest-event")]),
      revision: 3,
    });
    second.resolve({
      ...snapshotFixture([taskFixture("stale-response")]),
      revision: 2,
    });
    await next;
    expect(calls).toBe(2);
    expect(store.getState().taskIds).toEqual(["latest-event"]);
  });
});
