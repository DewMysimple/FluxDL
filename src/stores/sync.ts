import type { AppEvent, AppSnapshot, TaskDetailsResult } from "../lib/types";
import { errorText } from "../lib/utils";
import { mergeSnapshot, reduceChange, type AppCache } from "./cache";
import { isAppEvent } from "./protocol";

interface SyncOptions {
  read: () => AppCache;
  commit: (state: AppCache) => void;
  load: () => Promise<AppSnapshot>;
  reportError: (message: string) => void;
  recovered: () => void;
  maxEvents?: number;
  maxBytes?: number;
}
interface Buffered {
  event: AppEvent;
  bytes: number;
}
interface Request {
  epoch: number;
  serial: number;
  promise: Promise<void>;
}
const validVersion = (sessionId: string, revision: number) =>
  typeof sessionId === "string" &&
  sessionId.length > 0 &&
  sessionId.length <= 256 &&
  Number.isSafeInteger(revision) &&
  revision >= 0;

/** Protocol/lifecycle coordinator. No React, Tauri listener or task scheduling. */
export function createSyncController(options: SyncOptions) {
  const pending = new Map<string, Buffered>();
  const hints = new Map<string, number>();
  const retired = new Set<string>();
  const lostThrough = new Map<string, number>();
  let pendingBytes = 0;
  let serial = 0;
  // Malformed packets cannot be buffered safely, but an older in-flight read
  // must not erase the requirement to observe an authoritative state after them.
  let recoveryAfterSerial = 0;
  let epoch = 0;
  let active = true;
  let ready = true;
  let forced = true;
  let inFlight: Request | undefined;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let retryBudget = 1;
  let followupBudget = 2;
  const key = (event: AppEvent) => `${event.sessionId}\0${event.revision}`;
  const cancelRetry = () => {
    if (retry) clearTimeout(retry);
    retry = undefined;
  };
  function remove(bufferKey: string) {
    const item = pending.get(bufferKey);
    if (item) {
      pendingBytes -= item.bytes;
      pending.delete(bufferKey);
    }
  }
  function retire(sessionId: string) {
    retired.add(sessionId);
    if (retired.size > 8) retired.delete(retired.values().next().value ?? "");
    hints.delete(sessionId);
    lostThrough.delete(sessionId);
    for (const [bufferKey, item] of pending)
      if (item.event.sessionId === sessionId) remove(bufferKey);
  }
  function rememberHint(sessionId: string) {
    // A second observation can describe a session switch that occurred after
    // the in-flight snapshot was captured. Keep its latest observation.
    hints.set(sessionId, serial);
    if (hints.size > 4) {
      const oldest = hints.keys().next().value;
      if (oldest) {
        hints.delete(oldest);
        lostThrough.delete(oldest);
        for (const [bufferKey, item] of pending)
          if (item.event.sessionId === oldest) remove(bufferKey);
      }
    }
  }
  function confirmForeignSessions(sessionId: string, requestedAt: number) {
    for (const [candidate, lastSeen] of hints)
      if (candidate !== sessionId && lastSeen <= requestedAt) retire(candidate);
  }
  function needsRecovery() {
    const state = options.read();
    return (
      forced ||
      !state.sessionId ||
      hints.size > 0 ||
      pending.size > 0 ||
      (state.sessionId != null &&
        (lostThrough.get(state.sessionId) ?? -1) > state.revision)
    );
  }
  function add(event: AppEvent) {
    if (pending.has(key(event))) return;
    const bytes = new TextEncoder().encode(JSON.stringify(event)).length;
    pending.set(key(event), { event, bytes });
    pendingBytes += bytes;
    if (
      pending.size > (options.maxEvents ?? 256) ||
      pendingBytes > (options.maxBytes ?? 2 * 1024 * 1024)
    ) {
      for (const item of pending.values())
        lostThrough.set(
          item.event.sessionId,
          Math.max(
            lostThrough.get(item.event.sessionId) ?? 0,
            item.event.revision,
          ),
        );
      pending.clear();
      pendingBytes = 0;
      forced = true;
    }
  }
  function drain() {
    let state = options.read();
    for (const [bufferKey, item] of pending)
      if (
        item.event.sessionId === state.sessionId &&
        item.event.revision <= state.revision
      )
        remove(bufferKey);
    while (state.sessionId) {
      const bufferKey = `${state.sessionId}\0${state.revision + 1}`;
      const item = pending.get(bufferKey);
      if (!item) break;
      const result = reduceChange(state, item.event.change);
      if (result.needsSnapshot) {
        forced = true;
        break;
      }
      remove(bufferKey);
      options.commit({ ...result.state, revision: item.event.revision });
      state = options.read();
    }
  }
  function accept(snapshot: AppSnapshot, requestedAt = serial) {
    if (
      !active ||
      !validVersion(snapshot.sessionId, snapshot.revision) ||
      retired.has(snapshot.sessionId)
    )
      return false;
    const current = options.read();
    // The response still confirms its session when newer events have already
    // advanced the domain cache. It cannot exclude observations made later.
    confirmForeignSessions(snapshot.sessionId, requestedAt);
    if (
      snapshot.sessionId === current.sessionId &&
      snapshot.revision < current.revision
    ) {
      forced = requestedAt < recoveryAfterSerial;
      drain();
      if (!needsRecovery()) options.recovered();
      return false;
    }
    if (current.sessionId && snapshot.sessionId !== current.sessionId)
      retire(current.sessionId);
    options.commit(mergeSnapshot(current, snapshot));
    hints.delete(snapshot.sessionId);
    if (snapshot.revision >= (lostThrough.get(snapshot.sessionId) ?? -1))
      lostThrough.delete(snapshot.sessionId);
    forced = requestedAt < recoveryAfterSerial;
    drain();
    if (!needsRecovery()) {
      cancelRetry();
      options.recovered();
    }
    return true;
  }
  function request(external = true, expectedEpoch = epoch): Promise<void> {
    if (!active || expectedEpoch !== epoch) return Promise.resolve();
    if (external) {
      cancelRetry();
      retryBudget = 1;
      followupBudget = 2;
    }
    if (!ready) {
      forced = true;
      return Promise.resolve();
    }
    if (inFlight) return inFlight.promise;
    const token: Request = { epoch, serial, promise: Promise.resolve() };
    inFlight = token;
    token.promise = (async () => {
      let succeeded = false;
      try {
        const snapshot = await Promise.resolve().then(options.load);
        if (!active || token.epoch !== epoch || inFlight !== token) return;
        accept(snapshot, token.serial);
        succeeded = true;
      } catch (error) {
        if (active && token.epoch === epoch && inFlight === token) {
          forced = true;
          options.reportError(`无法同步本地状态：${errorText(error)}`);
          // Bounded I/O failure retry only; ordering is determined by versions.
          if (retryBudget > 0) {
            retryBudget--;
            retry = setTimeout(() => {
              retry = undefined;
              void request(false, token.epoch);
            }, 250);
          }
        }
      } finally {
        if (inFlight === token) {
          inFlight = undefined;
          if (succeeded && active && token.epoch === epoch && needsRecovery()) {
            if (followupBudget > 0) {
              followupBudget--;
              queueMicrotask(() => {
                if (active && epoch === token.epoch)
                  void request(false, token.epoch);
              });
            } else options.reportError("本地状态仍有缺口，请重新聚焦窗口刷新");
          }
        }
      }
    })();
    return token.promise;
  }
  function receive(input: unknown) {
    if (!active) return;
    serial++;
    if (!isAppEvent(input)) {
      forced = true;
      recoveryAfterSerial = serial;
      options.reportError("本地状态事件格式无效，正在重新同步");
      void request();
      return;
    }
    const event = input;
    if (retired.has(event.sessionId)) return;
    const state = options.read();
    if (event.sessionId !== state.sessionId) {
      rememberHint(event.sessionId);
      add(event);
    } else if (event.revision > state.revision) {
      if (event.revision === state.revision + 1) {
        const result = reduceChange(state, event.change);
        if (result.needsSnapshot) {
          add(event);
          forced = true;
        } else {
          options.commit({ ...result.state, revision: event.revision });
          drain();
        }
      } else add(event);
    }
    if (ready && needsRecovery()) void request();
  }
  function details(result: TaskDetailsResult) {
    if (!active || retired.has(result.sessionId)) return;
    const state = options.read();
    if (
      result.sessionId === state.sessionId &&
      result.revision < state.revision
    )
      return;
    if (
      result.sessionId === state.sessionId &&
      result.revision === state.revision
    ) {
      const index = state.taskIds.indexOf(result.task.id);
      if (index >= 0) {
        const merged = reduceChange(state, {
          type: "taskUpsert",
          task: result.task,
          index,
        });
        options.commit(merged.state);
        return;
      }
    }
    forced = true;
    serial++;
    recoveryAfterSerial = serial;
    if (validVersion(result.sessionId, result.revision)) {
      lostThrough.set(
        result.sessionId,
        Math.max(lostThrough.get(result.sessionId) ?? 0, result.revision),
      );
    }
    if (result.sessionId !== state.sessionId) rememberHint(result.sessionId);
    if (ready) void request();
  }
  return {
    receive,
    accept,
    details,
    refresh: request,
    getEpoch: () => epoch,
    resume: (expectedEpoch: number) => {
      if (active && expectedEpoch === epoch) ready = true;
    },
    begin: () => {
      epoch++;
      active = true;
      ready = false;
      inFlight = undefined;
      forced = true;
      pending.clear();
      pendingBytes = 0;
      hints.clear();
      lostThrough.clear();
      cancelRetry();
      const ownedEpoch = epoch;
      return () => {
        if (epoch !== ownedEpoch) return;
        active = false;
        ready = false;
        epoch++;
        inFlight = undefined;
        pending.clear();
        pendingBytes = 0;
        hints.clear();
        lostThrough.clear();
        cancelRetry();
      };
    },
    inspect: () => ({
      epoch,
      active,
      ready,
      inFlight: Boolean(inFlight),
      needsSnapshot: needsRecovery(),
      bufferedEvents: pending.size,
      bufferedBytes: pendingBytes,
      retiredSessions: [...retired],
    }),
  };
}
