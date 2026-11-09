import { create } from "zustand";
import { api, previewSnapshot } from "../lib/api";
import { errorText } from "../lib/utils";
import type { AppSnapshot, TaskDetailsResult } from "../lib/types";
import { createTaskDictionary, type AppCache } from "./cache";
import { createSyncController } from "./sync";

export interface AppStore extends AppCache {
  loading: boolean;
  error: string | null;
  errorSource: "operation" | "sync" | null;
  setSnapshot: (snapshot: AppSnapshot) => void;
  applyEvent: (event: unknown) => void;
  mergeDetails: (result: TaskDetailsResult) => void;
  refresh: () => Promise<void>;
  beginSync: (reconnect: () => Promise<void>) => () => void;
  inspectSync: () => ReturnType<
    ReturnType<typeof createSyncController>["inspect"]
  >;
  notify: (error: unknown) => void;
  clearError: () => void;
}

export function createAppStore(
  loadSnapshot: () => Promise<AppSnapshot> = () => api.snapshot(),
  limits?: { maxEvents?: number; maxBytes?: number },
) {
  let controller: ReturnType<typeof createSyncController>;
  let reconnect: (() => Promise<void>) | undefined;
  const store = create<AppStore>((set) => ({
    sessionId: null,
    revision: 0,
    settings: previewSnapshot.settings,
    engines: previewSnapshot.engines,
    tasksById: createTaskDictionary(),
    taskIds: [],
    listingRevision: 0,
    loading: true,
    error: null,
    errorSource: null,
    setSnapshot: (snapshot) => {
      controller.accept(snapshot);
    },
    applyEvent: (event) => controller.receive(event),
    mergeDetails: (result) => controller.details(result),
    inspectSync: () => controller.inspect(),
    refresh: async () => {
      const ownedEpoch = controller.getEpoch();
      const reconnectOwned = reconnect;
      if (reconnectOwned) await reconnectOwned();
      controller.resume(ownedEpoch);
      await controller.refresh(true, ownedEpoch);
    },
    beginSync: (callback) => {
      const stop = controller.begin();
      reconnect = callback;
      return () => {
        stop();
        if (reconnect === callback) reconnect = undefined;
      };
    },
    notify: (error) =>
      set({ error: errorText(error), errorSource: "operation" }),
    clearError: () => set({ error: null, errorSource: null }),
  }));
  controller = createSyncController({
    read: store.getState,
    commit: (next) =>
      store.setState((state) =>
        next === state && !state.loading ? state : { ...next, loading: false },
      ),
    load: loadSnapshot,
    reportError: (message) =>
      store.setState({ error: message, errorSource: "sync", loading: false }),
    recovered: () => {
      if (store.getState().errorSource === "sync")
        store.setState({ error: null, errorSource: null });
    },
    ...limits,
  });
  return store;
}
export const useApp = createAppStore();
export function toSnapshot(state: AppStore): AppSnapshot {
  return {
    sessionId: state.sessionId ?? previewSnapshot.sessionId,
    revision: state.revision,
    settings: state.settings,
    engines: state.engines,
    tasks: state.taskIds.map((id) => state.tasksById[id]),
  };
}
