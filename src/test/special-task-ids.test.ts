import { describe, expect, it } from "vitest";
import { createAppStore, toSnapshot } from "../stores/app";
import {
  eventFixture,
  mediaFixture,
  snapshotFixture,
  taskFixture,
} from "./fixtures";

describe("legacy arbitrary string task IDs", () => {
  it.each(["__proto__", "constructor", "toString"])(
    "retains and removes %s without inherited task lookup",
    (id) => {
      const store = createAppStore(() => new Promise(() => {}));
      expect(Object.getPrototypeOf(store.getState().tasksById)).toBeNull();
      expect(store.getState().tasksById[id]).toBeUndefined();
      const special = taskFixture(id, { status: "downloading" });
      const regular = taskFixture("ordinary", { status: "completed" });
      store.getState().setSnapshot(snapshotFixture([special, regular]));
      expect(store.getState().tasksById[id]).toBe(special);
      const ids = store.getState().taskIds;
      store.getState().applyEvent(
        eventFixture(1, {
          type: "taskProgress",
          taskId: id,
          status: "downloading",
          progress: 10,
          speed: null,
          eta: null,
          downloadedBytes: 1024,
          totalBytes: null,
          updatedAt: special.updatedAt,
          logsAppend: ["progress"],
        }),
      );
      expect(store.getState().tasksById[id].progress).toBe(10);
      expect(store.getState().taskIds).toBe(ids);
      expect(store.getState().tasksById.ordinary).toBe(regular);
      expect(Object.getPrototypeOf(store.getState().tasksById)).toBeNull();
      store.getState().applyEvent(
        eventFixture(2, {
          type: "taskUpsert",
          task: { ...special, media: mediaFixture({ title: "updated" }) },
          index: 0,
        }),
      );
      expect(store.getState().tasksById[id].media.title).toBe("updated");
      expect(Object.getPrototypeOf(store.getState().tasksById)).toBeNull();
      store.getState().applyEvent(
        eventFixture(3, {
          type: "fileAvailabilityUpdated",
          files: [{ taskId: id, fileExists: true }],
        }),
      );
      expect(store.getState().tasksById[id].fileExists).toBe(true);
      expect(store.getState().tasksById.ordinary).toBe(regular);
      expect(Object.getPrototypeOf(store.getState().tasksById)).toBeNull();
      store
        .getState()
        .applyEvent(eventFixture(4, { type: "taskRemoved", taskId: id }));
      expect(store.getState().taskIds).toEqual(["ordinary"]);
      expect(store.getState().tasksById[id]).toBeUndefined();
      expect(Object.getPrototypeOf(store.getState().tasksById)).toBeNull();
      // A detail response cannot resurrect a deleted record via inherited lookup.
      store.getState().mergeDetails({
        sessionId: "fixture-session",
        revision: 4,
        task: special,
      });
      expect(store.getState().tasksById[id]).toBeUndefined();
      expect(toSnapshot(store.getState()).tasks).toEqual([regular]);
      store.getState().beginSync(async () => undefined)();
    },
  );
});
