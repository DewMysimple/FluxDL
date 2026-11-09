import type {
  AppChange,
  AppSnapshot,
  DownloadTask,
  EngineDiagnostics,
  Settings,
  TaskStatus,
} from "../lib/types";
import { isActive } from "../lib/utils";

export interface AppCache {
  sessionId: string | null;
  revision: number;
  settings: Settings;
  engines: EngineDiagnostics;
  tasksById: Record<string, DownloadTask>;
  taskIds: string[];
  listingRevision: number;
}

/** Legacy task IDs are arbitrary strings, including Object prototype names. */
export function createTaskDictionary(
  source?: Readonly<Record<string, DownloadTask>>,
): Record<string, DownloadTask> {
  const dictionary: Record<string, DownloadTask> = {};
  Object.setPrototypeOf(dictionary, null);
  if (source) Object.assign(dictionary, source);
  return dictionary;
}

export function equivalent(left: unknown, right: unknown) {
  return left === right || JSON.stringify(left) === JSON.stringify(right);
}

function listingChanged(
  previous: DownloadTask | undefined,
  next: DownloadTask,
) {
  return (
    !previous ||
    previous.status !== next.status ||
    previous.media.title !== next.media.title ||
    previous.media.source !== next.media.source ||
    (next.status === "completed" && previous.updatedAt !== next.updatedAt)
  );
}

export function mergeSnapshot(
  state: AppCache,
  snapshot: AppSnapshot,
): AppCache {
  const ids = snapshot.tasks.map((task) => task.id);
  const idsChanged = !equivalent(state.taskIds, ids);
  let recordsChanged = idsChanged;
  let changedListing = idsChanged;
  const tasksById = createTaskDictionary();
  for (const task of snapshot.tasks) {
    const previous = state.tasksById[task.id];
    const same = previous && equivalent(previous, task);
    tasksById[task.id] = same ? previous : task;
    recordsChanged ||= !same;
    changedListing ||= listingChanged(previous, task);
  }
  const settings = equivalent(state.settings, snapshot.settings)
    ? state.settings
    : snapshot.settings;
  const engines = equivalent(state.engines, snapshot.engines)
    ? state.engines
    : snapshot.engines;
  if (
    !recordsChanged &&
    settings === state.settings &&
    engines === state.engines &&
    state.sessionId === snapshot.sessionId &&
    state.revision === snapshot.revision
  )
    return state;
  return {
    ...state,
    sessionId: snapshot.sessionId,
    revision: snapshot.revision,
    settings,
    engines,
    tasksById: recordsChanged ? tasksById : state.tasksById,
    taskIds: idsChanged ? ids : state.taskIds,
    listingRevision: state.listingRevision + Number(changedListing),
  };
}

function upsert(
  state: AppCache,
  task: DownloadTask,
  index: number,
): AppCache | null {
  const remaining = state.taskIds.filter((id) => id !== task.id);
  if (!Number.isSafeInteger(index) || index < 0 || index > remaining.length)
    return null;
  remaining.splice(index, 0, task.id);
  const sameOrder = equivalent(state.taskIds, remaining);
  const previous = state.tasksById[task.id];
  const sameTask = previous && equivalent(previous, task);
  if (sameOrder && sameTask) return state;
  const tasksById = sameTask
    ? state.tasksById
    : createTaskDictionary(state.tasksById);
  if (!sameTask) tasksById[task.id] = task;
  return {
    ...state,
    taskIds: sameOrder ? state.taskIds : remaining,
    tasksById,
    listingRevision:
      state.listingRevision +
      Number(!sameOrder || listingChanged(previous, task)),
  };
}

const phases: Partial<Record<TaskStatus, number>> = {
  queued: 0,
  resolving: 1,
  downloading: 2,
  postprocessing: 3,
  cancelling: 4,
};

/** Pure domain update. The controller owns session/sequence and recovery I/O. */
export function reduceChange(
  state: AppCache,
  change: AppChange,
): { state: AppCache; needsSnapshot: boolean } {
  let next = state;
  switch (change.type) {
    case "checkpoint":
      break;
    case "taskUpsert": {
      const result = upsert(state, change.task, change.index);
      return { state: result ?? state, needsSnapshot: !result };
    }
    case "taskProgress": {
      const task = state.tasksById[change.taskId];
      if (!task) return { state, needsSnapshot: true };
      // Old attempts are rejected by Rust before publishing. This extra guard
      // never turns a terminal record back into an active task via a small patch.
      if (!isActive(task.status)) break;
      if (!isActive(change.status)) return { state, needsSnapshot: true };
      const from = phases[task.status];
      const to = phases[change.status];
      if (from != null && to != null && to < from) break;
      const updated: DownloadTask = {
        ...task,
        status: change.status,
        progress: change.progress,
        speed: change.speed,
        eta: change.eta,
        downloadedBytes: change.downloadedBytes,
        totalBytes: change.totalBytes,
        updatedAt: change.updatedAt,
        logs: change.logsAppend.length
          ? [...task.logs, ...change.logsAppend].slice(-100)
          : task.logs,
      };
      const tasksById = createTaskDictionary(state.tasksById);
      tasksById[task.id] = updated;
      next = {
        ...state,
        tasksById,
        listingRevision:
          state.listingRevision + Number(listingChanged(task, updated)),
      };
      break;
    }
    case "taskRemoved": {
      if (!state.tasksById[change.taskId]) break;
      const tasksById = createTaskDictionary(state.tasksById);
      delete tasksById[change.taskId];
      next = {
        ...state,
        tasksById,
        taskIds: state.taskIds.filter((id) => id !== change.taskId),
        listingRevision: state.listingRevision + 1,
      };
      break;
    }
    case "settingsUpdated":
      if (!equivalent(state.settings, change.settings))
        next = { ...state, settings: change.settings };
      break;
    case "diagnosticsUpdated":
      if (!equivalent(state.engines, change.engines))
        next = { ...state, engines: change.engines };
      break;
    case "fileAvailabilityUpdated": {
      let tasksById = state.tasksById;
      for (const file of change.files) {
        const task = tasksById[file.taskId];
        if (!task || task.fileExists === file.fileExists) continue;
        if (tasksById === state.tasksById)
          tasksById = createTaskDictionary(tasksById);
        tasksById[file.taskId] = { ...task, fileExists: file.fileExists };
      }
      if (tasksById !== state.tasksById) next = { ...state, tasksById };
      break;
    }
    default:
      return { state, needsSnapshot: true };
  }
  return { state: next, needsSnapshot: false };
}
