import { eventFixture } from "./fixtures";
import { seedApp } from "./fixtures";
import { Profiler, StrictMode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import App from "../App";
import { api } from "../lib/api";
import { useApp } from "../stores/app";
import type { AppSnapshot, AppEvent } from "../lib/types";
import { installMatchMedia, snapshotFixture, taskFixture } from "./fixtures";

vi.mock("../lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("../lib/api")>();
  return {
    ...original,
    isDesktop: true,
    api: {
      ...original.api,
      snapshot: vi.fn(),
      onChange: vi.fn(),
      onClose: vi.fn(),
      onShutdownFailed: vi.fn(),
      onDiagnosticsTask: vi.fn(),
    },
  };
});

beforeEach(() => {
  vi.resetAllMocks();
  installMatchMedia();
  const snapshot = snapshotFixture();
  vi.mocked(api.snapshot).mockResolvedValue(snapshot);
  vi.mocked(api.onChange).mockResolvedValue(vi.fn());
  vi.mocked(api.onClose).mockResolvedValue(vi.fn());
  vi.mocked(api.onShutdownFailed).mockResolvedValue(vi.fn());
  vi.mocked(api.onDiagnosticsTask).mockResolvedValue(vi.fn());
  seedApp(snapshot, false);
});
afterEach(cleanup);

describe("desktop subscription lifecycle", () => {
  it("buffers an event delivered immediately during registration and replays it after the initial snapshot", async () => {
    useApp.setState({ sessionId: null });
    vi.mocked(api.onChange).mockImplementation(async (callback) => {
      callback(
        eventFixture(1, {
          type: "taskUpsert",
          task: taskFixture("instant", { status: "downloading" }),
          index: 0,
        }),
      );
      return () => undefined;
    });
    render(<App />);
    expect(
      await screen.findByRole("button", { name: "查看任务：媒体 instant" }),
    ).toBeTruthy();
    expect(useApp.getState().revision).toBe(1);
  });

  it("retries only failed listener registrations on focus and explicit recovery", async () => {
    vi.mocked(api.onChange)
      .mockRejectedValueOnce(new Error("暂时不能订阅"))
      .mockResolvedValue(vi.fn());
    render(<App />);
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "无法监听本地状态：暂时不能订阅",
    );
    await waitFor(() => expect(api.snapshot).toHaveBeenCalledTimes(1));
    await act(async () => window.dispatchEvent(new Event("focus")));
    await waitFor(() => expect(api.snapshot).toHaveBeenCalledTimes(2));
    await act(async () => useApp.getState().refresh());
    expect(api.onChange).toHaveBeenCalledTimes(2);
    expect(api.onClose).toHaveBeenCalledTimes(1);
    expect(api.onShutdownFailed).toHaveBeenCalledTimes(1);
  });

  it("contains an async unlisten rejection after unmount without writing a UI error", async () => {
    const warning = vi
      .spyOn(console, "warn")
      .mockImplementation(() => undefined);
    vi.mocked(api.onChange).mockResolvedValue(async () => {
      throw new Error("unlisten unavailable");
    });
    const view = render(<App />);
    await waitFor(() => expect(api.snapshot).toHaveBeenCalledTimes(1));
    await act(async () => view.unmount());
    expect(warning).toHaveBeenCalledWith(
      "FluxDL订阅清理失败：",
      "unlisten unavailable",
    );
    expect(useApp.getState().error).toBeNull();
    warning.mockRestore();
  });

  it("advances a healthy checkpoint without committing an unrelated React tree", async () => {
    const commits = vi.fn();
    render(
      <Profiler id="checkpoint" onRender={commits}>
        <App />
      </Profiler>,
    );
    await waitFor(() => expect(api.snapshot).toHaveBeenCalledTimes(1));
    await act(async () => {
      await Promise.resolve();
    });
    commits.mockClear();
    const callback = vi.mocked(api.onChange).mock.calls[0][0];
    await act(async () => callback(eventFixture(1, { type: "checkpoint" })));
    expect(useApp.getState().revision).toBe(1);
    expect(commits).not.toHaveBeenCalled();
  });

  it("requests its initial snapshot only after the event listeners are registered", async () => {
    const order: string[] = [];
    vi.mocked(api.onChange).mockImplementation(async () => {
      order.push("snapshot-listener");
      return () => undefined;
    });
    vi.mocked(api.onClose).mockImplementation(async () => {
      order.push("close-listener");
      return () => undefined;
    });
    vi.mocked(api.onShutdownFailed).mockImplementation(async () => {
      order.push("shutdown-listener");
      return () => undefined;
    });
    vi.mocked(api.snapshot).mockImplementation(async () => {
      order.push("snapshot-request");
      return snapshotFixture();
    });
    render(<App />);
    await waitFor(() =>
      expect(order).toEqual([
        "snapshot-listener",
        "close-listener",
        "shutdown-listener",
        "snapshot-request",
      ]),
    );
  });

  it("reports failed listener registration and still loads a usable snapshot", async () => {
    vi.mocked(api.onChange).mockRejectedValue(new Error("事件权限不足"));
    render(<App />);
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "无法监听本地状态：事件权限不足",
    );
    await waitFor(() => expect(api.snapshot).toHaveBeenCalledTimes(1));
  });

  it("keeps a newer event visible when the initial request returns late", async () => {
    let receive: ((event: AppEvent) => void) | undefined;
    let finish: (snapshot: AppSnapshot) => void = () => {
      throw new Error("Request not started");
    };
    vi.mocked(api.onChange).mockImplementation(async (callback) => {
      receive = callback;
      return () => undefined;
    });
    vi.mocked(api.snapshot).mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    render(<App />);
    await waitFor(() => expect(api.snapshot).toHaveBeenCalledTimes(1));
    await act(async () =>
      receive?.(
        eventFixture(1, {
          type: "taskUpsert",
          task: taskFixture("fresh", { status: "downloading" }),
          index: 0,
        }),
      ),
    );
    await act(async () =>
      finish(
        snapshotFixture([taskFixture("stale", { status: "downloading" })]),
      ),
    );
    expect(
      screen.getByRole("button", { name: "查看任务：媒体 fresh" }),
    ).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "查看任务：媒体 stale" }),
    ).toBeNull();
  });

  it("leaves one live subscription per event after StrictMode remount and none after unmount", async () => {
    const snapshots = new Set<(event: AppEvent) => void>();
    const closes = new Set<() => void>();
    const failures = new Set<(message: string) => void>();
    vi.mocked(api.onChange).mockImplementation(async (callback) => {
      const listener = (event: AppEvent) => callback(event);
      snapshots.add(listener);
      return () => {
        snapshots.delete(listener);
      };
    });
    vi.mocked(api.onClose).mockImplementation(async (callback) => {
      closes.add(callback);
      return () => {
        closes.delete(callback);
      };
    });
    vi.mocked(api.onShutdownFailed).mockImplementation(async (callback) => {
      failures.add(callback);
      return () => {
        failures.delete(callback);
      };
    });
    const view = render(
      <StrictMode>
        <App />
      </StrictMode>,
    );
    await waitFor(() =>
      expect([snapshots.size, closes.size, failures.size]).toEqual([1, 1, 1]),
    );
    await act(async () => {
      for (const callback of snapshots)
        callback(
          eventFixture(1, {
            type: "taskUpsert",
            task: taskFixture("event", { status: "downloading" }),
            index: 0,
          }),
        );
    });
    expect(
      screen.getByRole("button", { name: "查看任务：媒体 event" }),
    ).toBeTruthy();
    view.unmount();
    expect([snapshots.size, closes.size, failures.size]).toEqual([0, 0, 0]);
  });

  it("disposes a listener whose registration resolves after the window component unmounts", async () => {
    const resolvers: Array<(unlisten: () => void) => void> = [];
    const delayed = () =>
      new Promise<() => void>((resolve) => resolvers.push(resolve));
    vi.mocked(api.onChange).mockImplementation(delayed);
    vi.mocked(api.onClose).mockImplementation(delayed);
    vi.mocked(api.onShutdownFailed).mockImplementation(delayed);
    const view = render(<App />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(resolvers).toHaveLength(3);
    view.unmount();
    const unlisteners = resolvers.map(() => vi.fn());
    await act(async () =>
      resolvers.forEach((resolve, index) => resolve(unlisteners[index])),
    );
    for (const unlisten of unlisteners)
      expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("refreshes on focus only while mounted and reports a shutdown failure", async () => {
    let fail: ((message: string) => void) | undefined;
    vi.mocked(api.onShutdownFailed).mockImplementation(async (callback) => {
      fail = callback;
      return () => undefined;
    });
    const view = render(<App />);
    await waitFor(() => expect(api.snapshot).toHaveBeenCalledTimes(1));
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(api.snapshot).toHaveBeenCalledTimes(2);
    await act(async () => fail?.("无法安全退出：请重试"));
    expect(screen.getByRole("alert").textContent).toContain(
      "无法安全退出：请重试",
    );
    view.unmount();
    window.dispatchEvent(new Event("focus"));
    expect(api.snapshot).toHaveBeenCalledTimes(2);
  });
});
